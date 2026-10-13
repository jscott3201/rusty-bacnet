//! Network-layer messages taken by the router (router/control_messages.rs),
//! and the RB-09 gate in front of the protected ones.

mod fixture;

use bacnet_router_core::message::{MessageType, RejectReason};
use bacnet_router_core::npdu::Npdu;
use bacnet_router_core::router::{ControlClass, Discard, Event, Kind, Options};
use bacnet_router_core::table::{Convergence, Reachability};
use bacnet_router_core::{Mac, PortId};
use fixture::{
    addr, at, bytes, deliver, deliver_broadcast, i_am, learn_9_via_r7, listed, message, networks,
    options, port_on, router, router_with, sent_reject, Outcome, Recorder, NODE_A, OWN_0, OWN_1,
    R7,
};

/// Another router on port 1.
const R8: &[u8] = &[0x08];

/// Clauses 6.5.2.2 and 6.6.3.5: a message type the router doesn't handle,
/// arriving for it, draws reason 3 naming network 0; the removed security
/// types are taken quietly (control_messages.rs:787-808).
#[test]
fn unknown_types_draw_reason_3() {
    let mut router = router();
    let mut host = Recorder::default();
    let mut proprietary = Npdu::message(MessageType::from_raw(0x80), &[1]);
    proprietary.vendor_id = Some(260);
    for (kind, frame) in [
        (0x14, message(MessageType::from_raw(0x14), &[])),
        (0x80, bytes(&proprietary)),
    ] {
        let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &frame, at(1));
        assert_eq!(outcome, Outcome::Handled);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].to.as_deref(), Some(NODE_A));
        assert_eq!(
            sent_reject(&sent[0]),
            (RejectReason::UNKNOWN_MESSAGE_TYPE, 0, None)
        );
        assert_eq!(
            host.events.pop(),
            Some(Event::UnknownMessage {
                message_type: MessageType::from_raw(kind)
            })
        );
    }
    let (outcome, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &message(MessageType::from_raw(0x0A), &[]),
        at(1),
    );
    assert_eq!(outcome, Outcome::Handled);
    assert!(sent.is_empty());
    assert_eq!(
        host.events,
        [Event::SecurityMessage {
            message_type: MessageType::from_raw(0x0A)
        }]
    );
}

/// Clauses 6.6.3.2 and 6.6.3.3: every network an I-Am-Router lists is
/// learned through the arrival port and its sender, and the message goes
/// on to every other port as it came. A malformed or refused one does
/// neither.
#[test]
fn i_am_router_learns_then_rebroadcasts() {
    let ports = [port_on(1, OWN_0), port_on(2, OWN_1), port_on(3, &[0x33])];
    let mut router = router_with(&ports, Options::HOSTED);
    let mut host = Recorder::default();
    let mut frame = Npdu::message(MessageType::I_AM_ROUTER_TO_NETWORK, &[0, 9, 0, 10]);
    frame.priority = 2;
    let (outcome, sent) = deliver_broadcast(&mut router, &mut host, 1, R7, &bytes(&frame), at(1));
    assert_eq!(outcome, Outcome::Handled);
    for network in [9, 10] {
        let route = router.table().lookup(network).unwrap();
        assert_eq!((route.port(), route.next_hop().as_slice()), (PortId(1), R7));
    }
    let ports: Vec<u8> = sent.iter().map(|sent| sent.port).collect();
    assert_eq!(ports, [0, 2]);
    for sent in &sent {
        assert_eq!(sent.kind, Kind::Relay(MessageType::I_AM_ROUTER_TO_NETWORK));
        // Re-sent as a fresh message: data verbatim, priority normal.
        assert_eq!(sent.npdu, i_am(&[9, 10]));
    }
    assert_eq!(host.asked.len(), 1);
    assert_eq!(
        (host.asked[0].class, host.asked[0].targets.as_slice()),
        (ControlClass::IAm, &[9, 10][..])
    );

    for bad in [
        message(MessageType::I_AM_ROUTER_TO_NETWORK, &[]),
        message(MessageType::I_AM_ROUTER_TO_NETWORK, &[0, 44, 0]),
    ] {
        let (outcome, sent) = deliver_broadcast(&mut router, &mut host, 1, R7, &bad, at(2));
        assert_eq!(outcome, Outcome::Discarded(Discard::BadMessage));
        assert!(sent.is_empty());
    }
    host.deny.push(ControlClass::IAm);
    let (outcome, sent) = deliver_broadcast(&mut router, &mut host, 1, R7, &i_am(&[44]), at(3));
    assert_eq!(outcome, Outcome::Discarded(Discard::Denied));
    assert!(sent.is_empty());
    assert!(router.table().lookup(44).is_none());
}

/// A network listed twice in one message is one claim: under hardened
/// convergence the repeat can't corroborate the first
/// (control_messages.rs:253-261).
#[test]
fn one_message_never_corroborates_itself() {
    let options = options(|o| o.convergence = Convergence::Hardened);
    let mut router = router_with(&fixture::two_ports(), options);
    let mut host = Recorder::default();
    learn_9_via_r7(&mut router, at(0));
    deliver_broadcast(&mut router, &mut host, 0, NODE_A, &i_am(&[9, 9]), at(1));
    assert_eq!(router.table().lookup(9).unwrap().port(), PortId(1));
    assert_eq!(router.table().claim_counters().pending_started, 1);
    assert_eq!(router.table().claim_counters().corroborated_applied, 0);

    // A repeat after a lower network is still one claim: the set of
    // networks claimed stays sorted however the list runs.
    let mut router = router_with(&fixture::two_ports(), options);
    learn_9_via_r7(&mut router, at(0));
    let r7 = Mac::from_slice(R7).unwrap();
    assert!(router.table_mut().add_learned(8, PortId(1), r7, at(0)));
    deliver_broadcast(&mut router, &mut host, 0, NODE_A, &i_am(&[9, 8, 9]), at(1));
    assert_eq!(router.table().lookup(9).unwrap().port(), PortId(1));
    assert_eq!(router.table().claim_counters().pending_started, 2);
    assert_eq!(router.table().claim_counters().corroborated_applied, 0);

    // In standard mode the first moves it and the repeat refreshes it.
    let mut router = fixture::router();
    learn_9_via_r7(&mut router, at(0));
    deliver_broadcast(&mut router, &mut host, 0, NODE_A, &i_am(&[9, 9]), at(1));
    assert_eq!(router.table().lookup(9).unwrap().port(), PortId(0));
    assert_eq!(router.table().claim_counters().learned_ok, 2);
}

/// The route cap: an I-Am-Router stops learning new networks there but
/// still refreshes ones the table holds; an Init-ACK learns nothing more,
/// as today (control_messages.rs:247 against :762). Direct routes count.
#[test]
fn the_route_cap_ends_learning() {
    let options = options(|o| o.route_cap = 4);
    let mut router = router_with(&fixture::two_ports(), options);
    let mut host = Recorder::default();
    deliver_broadcast(&mut router, &mut host, 1, R7, &i_am(&[9, 10, 11]), at(1));
    assert_eq!(router.table().networks().collect::<Vec<_>>(), [1, 2, 9, 10]);
    assert!(host.events.contains(&Event::RouteCap {
        class: ControlClass::IAm
    }));
    // At the cap, a held network still refreshes, until a new one ends it.
    deliver_broadcast(&mut router, &mut host, 1, R8, &i_am(&[10, 12, 9]), at(2));
    assert_eq!(router.table().lookup(10).unwrap().next_hop().as_slice(), R8);
    assert_eq!(router.table().lookup(9).unwrap().next_hop().as_slice(), R7);
    // An Init-ACK at the cap stops at once, even for a held network.
    let ack = message(MessageType::INITIALIZE_ROUTING_TABLE_ACK, &[1, 0, 9, 1, 0]);
    deliver(&mut router, &mut host, 1, R8, &ack, at(3));
    assert_eq!(router.table().lookup(9).unwrap().next_hop().as_slice(), R7);
    let counters = router.table().claim_counters();
    assert_eq!((counters.learned_ok, counters.learned_cap_ignored), (3, 3));
}

/// Clause 6.6.3.2, scoped: answered with the network when it lies beyond
/// another port, whatever its state; searched for further, with SNET and
/// SADR and no DNET, when the table lacks it; and left to the
/// router on the arrival network when the table reaches it through the
/// arrival port. Network 0, and data of the wrong length, are refused.
#[test]
fn scoped_who_is_router_cases() {
    let mut router = router();
    let mut host = Recorder::default();
    learn_9_via_r7(&mut router, at(0));
    router.table_mut().mark_unreachable(9);
    let who_is = |network: u16| {
        message(
            MessageType::WHO_IS_ROUTER_TO_NETWORK,
            &network.to_be_bytes(),
        )
    };

    let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &who_is(9), at(1));
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(sent.len(), 1);
    assert_eq!((sent[0].port, sent[0].is_broadcast()), (0, true));
    assert_eq!(
        sent[0].kind,
        Kind::Answer(MessageType::I_AM_ROUTER_TO_NETWORK)
    );
    assert_eq!(listed(&sent[0]), [9]);

    // Unknown: searched for on port 1, with A as the origin.
    let mut asked = Npdu::message(MessageType::WHO_IS_ROUTER_TO_NETWORK, &[0, 44]);
    asked.priority = 1;
    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &bytes(&asked), at(1));
    assert_eq!(sent.len(), 1);
    assert_eq!((sent[0].port, sent[0].is_broadcast()), (1, true));
    assert_eq!(
        sent[0].kind,
        Kind::Relay(MessageType::WHO_IS_ROUTER_TO_NETWORK)
    );
    let expected = Npdu {
        source: Some(addr(1, NODE_A)),
        ..asked
    };
    assert_eq!(sent[0].npdu, bytes(&expected));
    // A request a router passed on keeps its origin.
    let mut routed = Npdu::message(MessageType::WHO_IS_ROUTER_TO_NETWORK, &[0, 44]);
    routed.source = Some(addr(7, &[1]));
    let (_, sent) = deliver(&mut router, &mut host, 1, R8, &bytes(&routed), at(1));
    assert_eq!(sent[0].port, 0);
    assert_eq!(sent[0].parsed().source, Some(addr(7, &[1])));

    // Reached through the arrival port, as 9 is behind R7 and 2 is port
    // 1's own network, so the router there answers: nothing from this one.
    for network in [9, 2] {
        let (outcome, sent) = deliver(&mut router, &mut host, 1, R8, &who_is(network), at(1));
        assert_eq!(outcome, Outcome::Handled);
        assert!(sent.is_empty(), "{network}");
    }

    for bad in [
        who_is(0),
        message(MessageType::WHO_IS_ROUTER_TO_NETWORK, &[0, 9, 0]),
    ] {
        let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &bad, at(1));
        assert_eq!(outcome, Outcome::Discarded(Discard::BadMessage));
        assert!(sent.is_empty());
    }
    assert!(host.asked.is_empty(), "discovery is never gated");
}

/// Clause 6.6.3.2, unscoped: every network the table reaches
/// through another port, ascending, except one marked unreachable; a busy
/// one stays. A router with nothing to list sends nothing.
#[test]
fn unscoped_answer_lists_reachable_networks() {
    let ports = [port_on(1, OWN_0), port_on(2, OWN_1), port_on(3, &[0x33])];
    let who_is_all = message(MessageType::WHO_IS_ROUTER_TO_NETWORK, &[]);
    let mut router = router_with(&ports, Options::HOSTED);
    let mut host = Recorder::default();
    let r8 = Mac::from_slice(R8).unwrap();
    router.table_mut().add_learned(10, PortId(1), r8, at(0));
    learn_9_via_r7(&mut router, at(0));
    router.table_mut().mark_unreachable(9);
    router.table_mut().mark_busy(10, at(30));
    let mac = Mac::from_slice(NODE_A).unwrap();
    router.table_mut().add_learned(5, PortId(0), mac, at(0));
    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &who_is_all, at(1));
    assert_eq!(sent.len(), 1);
    assert_eq!((sent[0].port, sent[0].is_broadcast()), (0, true));
    assert_eq!(
        sent[0].kind,
        Kind::Answer(MessageType::I_AM_ROUTER_TO_NETWORK)
    );
    assert_eq!(listed(&sent[0]), [2, 3, 10]);

    let mut lonely = router_with(&[port_on(1, OWN_0)], Options::HOSTED);
    let mut host = Recorder::default();
    let (outcome, sent) = deliver(&mut lonely, &mut host, 0, NODE_A, &who_is_all, at(1));
    assert_eq!(outcome, Outcome::Handled);
    assert!(sent.is_empty());
}

/// Clauses 6.4.5, 6.4.6, 6.6.3.6 and 6.6.3.7: flow control covers what the
/// sender serves, learned routes through the arrival port with it as next
/// hop, narrowed by a list; direct routes never; and it goes on regardless
/// (control_messages.rs:324-447).
#[test]
fn flow_control_covers_what_the_sender_serves() {
    let mut router = router();
    let mut host = Recorder::default();
    learn_9_via_r7(&mut router, at(0));
    let r8 = Mac::from_slice(R8).unwrap();
    router.table_mut().add_learned(4000, PortId(1), r8, at(0));
    let busy = |list: &[u16]| message(MessageType::ROUTER_BUSY_TO_NETWORK, &networks(list));

    // R7's list names 9, R8's 4000, and direct 2: only 9 is R7's.
    let (_, sent) = deliver_broadcast(&mut router, &mut host, 1, R7, &busy(&[9, 4000, 2]), at(1));
    assert_eq!(
        router.table().lookup(9).unwrap().reachability(),
        Reachability::Busy
    );
    assert_eq!(
        router.table().lookup(4000).unwrap().reachability(),
        Reachability::Reachable
    );
    assert_eq!(
        router.table().lookup(2).unwrap().reachability(),
        Reachability::Reachable
    );
    assert_eq!(listed(&sent[0]), [9, 4000, 2]);

    // A sender that serves nothing marks nothing, and is passed on anyway.
    let (outcome, sent) = deliver_broadcast(&mut router, &mut host, 1, &[0x55], &busy(&[]), at(2));
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(sent.len(), 1);
    assert_eq!(
        router.table().lookup(4000).unwrap().reachability(),
        Reachability::Reachable
    );
    let last = host.asked.last().unwrap();
    assert!(last.targets_omitted && last.targets.is_empty());

    // Busy never revives an unreachable route; Available lifts it.
    router.table_mut().mark_unreachable(4000);
    deliver_broadcast(&mut router, &mut host, 1, R8, &busy(&[]), at(3));
    assert_eq!(
        router.table().lookup(4000).unwrap().reachability(),
        Reachability::Unreachable
    );
    let available = message(MessageType::ROUTER_AVAILABLE_TO_NETWORK, &[]);
    deliver_broadcast(&mut router, &mut host, 1, R8, &available, at(4));
    assert_eq!(
        router.table().lookup(4000).unwrap().reachability(),
        Reachability::Reachable
    );

    // Refused or malformed: no marks, nothing passed on.
    host.deny.push(ControlClass::Busy);
    let (outcome, sent) = deliver_broadcast(&mut router, &mut host, 1, R8, &busy(&[4000]), at(5));
    assert_eq!(outcome, Outcome::Discarded(Discard::Denied));
    assert!(sent.is_empty());
    let odd = message(MessageType::ROUTER_AVAILABLE_TO_NETWORK, &[0]);
    let (outcome, sent) = deliver_broadcast(&mut router, &mut host, 1, R8, &odd, at(5));
    assert_eq!(outcome, Outcome::Discarded(Discard::BadMessage));
    assert!(sent.is_empty());
    assert_eq!(
        router.table().lookup(4000).unwrap().reachability(),
        Reachability::Reachable
    );
    let marks = router.table().lookup(9).unwrap();
    assert_eq!(
        marks.last_seen(),
        Some(at(0)),
        "marks leave freshness alone"
    );
}

/// Clauses 6.4.19 and 6.4.20: What-Is and Number-Is are never routed. A
/// What-Is with no addresses and no data is answered on its port with the
/// port's configured number; a Number-Is is heard only as a link broadcast
/// with no addresses, and a different number is reported.
#[test]
fn network_number_messages_stay_local() {
    let mut router = router();
    let mut host = Recorder::default();
    let what_is = message(MessageType::WHAT_IS_NETWORK_NUMBER, &[]);
    let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &what_is, at(1));
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(sent.len(), 1);
    assert_eq!((sent[0].port, sent[0].is_broadcast()), (1, true));
    assert_eq!(sent[0].kind, Kind::Answer(MessageType::NETWORK_NUMBER_IS));
    assert_eq!(
        sent[0].npdu,
        message(MessageType::NETWORK_NUMBER_IS, &[0, 2, 1])
    );

    learn_9_via_r7(&mut router, at(0));
    let mut directed = Npdu::message(MessageType::WHAT_IS_NETWORK_NUMBER, &[]);
    directed.destination = Some(addr(9, &[5]));
    let with_data = Npdu::message(MessageType::WHAT_IS_NETWORK_NUMBER, &[0]);
    let mut number_routed = Npdu::message(MessageType::NETWORK_NUMBER_IS, &[0, 1, 1]);
    number_routed.source = Some(addr(7, &[1]));
    for frame in [bytes(&directed), bytes(&with_data), bytes(&number_routed)] {
        let (outcome, sent) = deliver_broadcast(&mut router, &mut host, 0, NODE_A, &frame, at(2));
        assert_eq!(outcome, Outcome::Discarded(Discard::BadMessage));
        assert!(sent.is_empty(), "never routed");
    }
    let number = message(MessageType::NETWORK_NUMBER_IS, &[0, 5, 1]);
    let (outcome, _) = deliver(&mut router, &mut host, 0, NODE_A, &number, at(3));
    assert_eq!(
        outcome,
        Outcome::Discarded(Discard::BadMessage),
        "a unicast"
    );
    let (outcome, sent) = deliver_broadcast(&mut router, &mut host, 0, NODE_A, &number, at(3));
    assert_eq!(outcome, Outcome::Handled);
    assert!(sent.is_empty());
    assert_eq!(
        host.events,
        [Event::NumberConflict {
            port: PortId(0),
            local: 1,
            reported: 5,
            configured: true
        }]
    );
    let same = message(MessageType::NETWORK_NUMBER_IS, &[0, 1, 0]);
    deliver_broadcast(&mut router, &mut host, 0, NODE_A, &same, at(4));
    assert_eq!(host.events.len(), 1);
}

/// Clauses 6.6.3.10 and 6.6.3.11: with no point-to-point links, Establish-
/// and Disconnect-Connection change nothing (control_messages.rs:643-666).
#[test]
fn ptp_messages_change_nothing() {
    let mut router = router();
    let mut host = Recorder::default();
    learn_9_via_r7(&mut router, at(0));
    let before = router.table().lookup(9);
    let establish = message(MessageType::ESTABLISH_CONNECTION_TO_NETWORK, &[0, 9, 5]);
    let disconnect = message(MessageType::DISCONNECT_CONNECTION_TO_NETWORK, &[0, 9]);
    for frame in [&establish, &disconnect] {
        let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, frame, at(1));
        assert_eq!(outcome, Outcome::Handled);
        assert!(sent.is_empty());
    }
    assert_eq!(router.table().lookup(9), before);
    assert_eq!(
        host.events,
        [
            Event::EstablishConnection {
                network: 9,
                termination_time: 5
            },
            Event::DisconnectIgnored { network: 9 },
        ]
    );
    assert_eq!(
        router.table().claim_counters().disconnect_removal_ignored,
        1
    );
    for bad in [
        message(MessageType::ESTABLISH_CONNECTION_TO_NETWORK, &[0, 9]),
        message(MessageType::DISCONNECT_CONNECTION_TO_NETWORK, &[0, 9, 0]),
    ] {
        let (outcome, _) = deliver(&mut router, &mut host, 1, R7, &bad, at(1));
        assert_eq!(outcome, Outcome::Discarded(Discard::BadMessage));
    }
    assert_eq!(
        router.table().claim_counters().disconnect_removal_ignored,
        1
    );
}

/// Clause 6.4.3: an I-Could-Be-Router is stored as a learned route only
/// where the table has none (control_messages.rs:617-642).
#[test]
fn i_could_be_router_fills_only_gaps() {
    let mut router = router();
    let mut host = Recorder::default();
    learn_9_via_r7(&mut router, at(0));
    let could_be = |network: u16| {
        let [high, low] = network.to_be_bytes();
        message(MessageType::I_COULD_BE_ROUTER_TO_NETWORK, &[high, low, 7])
    };
    deliver(&mut router, &mut host, 1, R8, &could_be(9), at(1));
    deliver(&mut router, &mut host, 1, R8, &could_be(44), at(1));
    assert_eq!(router.table().lookup(9).unwrap().next_hop().as_slice(), R7);
    assert_eq!(router.table().lookup(44).unwrap().next_hop().as_slice(), R8);
    assert_eq!(
        host.events,
        [
            Event::ICouldBe {
                network: 9,
                performance_index: 7,
                stored: false
            },
            Event::ICouldBe {
                network: 44,
                performance_index: 7,
                stored: true
            },
        ]
    );
    assert_eq!(host.asked[1].targets, [44]);
    host.deny.push(ControlClass::ICouldBe);
    let (outcome, _) = deliver(&mut router, &mut host, 1, R8, &could_be(45), at(1));
    assert_eq!(outcome, Outcome::Discarded(Discard::Denied));
    assert!(router.table().lookup(45).is_none());
}

/// The route cap turns an I-Could-Be-Router for a new network away, and
/// counts it as the cap's other refusals are; reserved networks and ones
/// the table holds are no new network, so they don't count. The hosted
/// router stored these past its cap before #1679.
#[test]
fn the_route_cap_turns_i_could_be_router_away() {
    let options = options(|o| o.route_cap = 3);
    let mut router = router_with(&fixture::two_ports(), options);
    let mut host = Recorder::default();
    let could_be = |network: u16| {
        let [high, low] = network.to_be_bytes();
        message(MessageType::I_COULD_BE_ROUTER_TO_NETWORK, &[high, low, 7])
    };
    deliver(&mut router, &mut host, 1, R8, &could_be(44), at(1));
    assert!(router.table().lookup(44).is_some());
    for network in [45, 44, 0, 0xFFFF] {
        let (outcome, sent) = deliver(&mut router, &mut host, 1, R8, &could_be(network), at(2));
        assert_eq!(outcome, Outcome::Handled, "{network}");
        assert!(sent.is_empty(), "{network}");
    }
    assert_eq!(router.table().networks().collect::<Vec<_>>(), [1, 2, 44]);
    assert_eq!(router.table().claim_counters().learned_cap_ignored, 1);
    assert_eq!(
        host.events[1..3],
        [
            Event::RouteCap {
                class: ControlClass::ICouldBe
            },
            Event::ICouldBe {
                network: 45,
                performance_index: 7,
                stored: false
            },
        ]
    );
    assert_eq!(host.events.len(), 6, "{:?}", host.events);
}

/// RB-09: the host sees each protected message once, after its data is
/// read and before anything changes, with what it claims; read-only ones
/// never reach it.
#[test]
fn the_gate_sees_each_protected_class_with_its_claims() {
    let mut router = router();
    let mut host = Recorder::default();
    learn_9_via_r7(&mut router, at(0));
    let mut routed = Npdu::message(MessageType::ROUTER_BUSY_TO_NETWORK, &[0, 9]);
    routed.source = Some(addr(7, &[1]));
    routed.destination = Some(addr(2, &[]));
    let frames = [
        (ControlClass::IAm, i_am(&[44])),
        (ControlClass::Busy, bytes(&routed)),
        (
            ControlClass::Available,
            message(MessageType::ROUTER_AVAILABLE_TO_NETWORK, &[0, 9]),
        ),
        (
            ControlClass::Reject,
            message(MessageType::REJECT_MESSAGE_TO_NETWORK, &[1, 0, 9]),
        ),
        (
            ControlClass::ICouldBe,
            message(MessageType::I_COULD_BE_ROUTER_TO_NETWORK, &[0, 45, 1]),
        ),
        (
            ControlClass::InitMgmt,
            message(MessageType::INITIALIZE_ROUTING_TABLE, &[1, 0, 46, 2, 0]),
        ),
        (
            ControlClass::InitAck,
            message(MessageType::INITIALIZE_ROUTING_TABLE_ACK, &[1, 0, 47, 1, 0]),
        ),
    ];
    for (class, frame) in &frames {
        deliver_broadcast(&mut router, &mut host, 1, R7, frame, at(1));
        let asked = host.asked.last().unwrap();
        assert_eq!(asked.class, *class);
        assert_eq!(
            (asked.port, asked.port_network, asked.source.as_slice()),
            (PortId(1), 2, R7)
        );
    }
    assert_eq!(host.asked.len(), frames.len());
    let busy = &host.asked[1];
    assert_eq!(
        (busy.snet, busy.dnet, busy.targets.as_slice()),
        (Some(7), Some(2), &[9][..])
    );
    assert_eq!(busy.message_type, MessageType::ROUTER_BUSY_TO_NETWORK);
    assert_eq!(host.asked[5].targets, [46]);

    for read_only in [
        message(MessageType::WHO_IS_ROUTER_TO_NETWORK, &[]),
        message(MessageType::INITIALIZE_ROUTING_TABLE, &[0]),
        message(MessageType::WHAT_IS_NETWORK_NUMBER, &[]),
        message(MessageType::DISCONNECT_CONNECTION_TO_NETWORK, &[0, 9]),
    ] {
        deliver_broadcast(&mut router, &mut host, 1, R7, &read_only, at(2));
    }
    assert_eq!(host.asked.len(), frames.len());
    assert_eq!(router.counters().denied, 0);

    // Local writes through the table are never asked about.
    let mac = Mac::from_slice(R8).unwrap();
    router.table_mut().add_learned(48, PortId(1), mac, at(3));
    assert_eq!(host.asked.len(), frames.len());
}

/// Directed elsewhere: routed, or refused like an APDU when there is no
/// route; for the arrival network or 0xFFFF: handled here.
#[test]
fn messages_for_other_networks_are_routed_first() {
    let mut router = router();
    let mut host = Recorder::default();
    let mut directed = Npdu::message(MessageType::I_AM_ROUTER_TO_NETWORK, &[0, 44]);
    directed.destination = Some(addr(77, &[5]));
    let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &bytes(&directed), at(1));
    assert_eq!(outcome, Outcome::Handled);
    let kinds: Vec<Kind> = sent.iter().map(|sent| sent.kind).collect();
    assert_eq!(
        kinds,
        [
            Kind::Solicit,
            Kind::Reject(RejectReason::NOT_DIRECTLY_CONNECTED)
        ]
    );
    assert!(router.table().lookup(44).is_none());

    for dnet in [1, 0xFFFF] {
        directed.destination = Some(addr(dnet, &[]));
        let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &bytes(&directed), at(10));
        assert_eq!(
            sent[0].kind,
            Kind::Relay(MessageType::I_AM_ROUTER_TO_NETWORK)
        );
    }
    assert!(router.table().lookup(44).is_some());
}
