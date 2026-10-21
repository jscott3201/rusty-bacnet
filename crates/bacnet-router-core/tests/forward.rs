//! Rewriting for the next hop (Clause 6.5.4; router/forwarding.rs), the
//! global fan-out, and the egress limits (#1665).

mod fixture;

use bacnet_router_core::npdu::NetworkPriority;
use bacnet_router_core::npdu::Npdu;
use bacnet_router_core::npdu::NpduLimits;
use bacnet_router_core::npdu::{NetworkMessageType, RejectMessageReason};
use bacnet_router_core::router::{Cause, Discard, Event, Kind, Options};
use bacnet_router_core::PortId;
use fixture::{
    addr, apdu_to, at, bytes, deliver, learn_9_via_r7, port_on, router, router_with, sent_reject,
    two_ports, Outcome, Recorder, CONFIRMED, NODE_A, OWN_0, OWN_1, R7, UNCONFIRMED,
};

/// Clause 6.5.4: SNET and SADR are added from the arrival network and the
/// link source when missing, and kept when present.
#[test]
fn source_is_added_once_and_kept_after() {
    let mut router = router();
    let mut host = Recorder::default();
    let (_, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &apdu_to(2, &[5], CONFIRMED),
        at(1),
    );
    assert_eq!(sent[0].parsed().source, Some(addr(1, NODE_A)));

    let mut relayed = Npdu::apdu(CONFIRMED);
    relayed.destination = Some(addr(2, &[5]));
    relayed.source = Some(addr(7, &[1, 2, 3]));
    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &bytes(&relayed), at(1));
    assert_eq!(sent[0].parsed().source, Some(addr(7, &[1, 2, 3])));
}

/// Clause 6.5.4: onto a directly attached network the DNET, DADR and hop
/// count go, and the NPDU goes to the DADR, or as a broadcast for DLEN 0.
/// Priority, expecting-reply and the APDU travel unchanged.
#[test]
fn direct_delivery_strips_destination() {
    let mut router = router();
    let mut host = Recorder::default();
    let mut request = Npdu::apdu(CONFIRMED);
    request.destination = Some(addr(2, &[5]));
    request.hop_count = 9;
    request.priority = NetworkPriority::LifeSafety;
    request.expecting_reply = true;
    let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &bytes(&request), at(1));
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(sent.len(), 1);
    assert_eq!((sent[0].port, sent[0].to.as_deref()), (1, Some(&[5][..])));
    assert_eq!(
        (sent[0].kind, sent[0].cause, sent[0].priority),
        (Kind::Forward, Cause::Ingress, NetworkPriority::LifeSafety)
    );
    let expected = Npdu {
        destination: None,
        source: Some(addr(1, NODE_A)),
        ..request
    };
    assert_eq!(sent[0].npdu, bytes(&expected));

    let (_, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &apdu_to(2, &[], UNCONFIRMED),
        at(1),
    );
    assert_eq!((sent[0].port, sent[0].is_broadcast()), (1, true));
    assert_eq!(sent[0].parsed().destination, None);
}

/// Clause 6.5.4 and 6.2.3: toward another router the DNET and DADR stay,
/// the NPDU goes to that router, and the hop count drops by one.
#[test]
fn next_router_keeps_destination_and_counts_down() {
    let mut router = router();
    let mut host = Recorder::default();
    learn_9_via_r7(&mut router, at(0));
    let mut request = Npdu::apdu(CONFIRMED);
    request.destination = Some(addr(9, &[5]));
    request.hop_count = 17;
    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &bytes(&request), at(1));
    assert_eq!(sent[0].to.as_deref(), Some(R7));
    let parsed = sent[0].parsed();
    assert_eq!(parsed.destination, Some(addr(9, &[5])));
    assert_eq!(parsed.hop_count, 16);
}

/// Clauses 6.2.3 and 6.5.4: the hop count comes down before the
/// router decides, so toward the next router one that arrives with 0 or 1
/// goes nowhere, counted and reported, with no reject; 2 leaves with 1.
/// Onto an attached network the NPDU carries no hop count, so one that
/// arrives with 1 is still delivered there.
#[test]
fn hop_count_reaching_zero_is_not_relayed() {
    let mut router = router();
    let mut host = Recorder::default();
    learn_9_via_r7(&mut router, at(0));
    let hops = |network: u16, count: u8| {
        let mut request = Npdu::apdu(CONFIRMED);
        request.destination = Some(addr(network, &[5]));
        request.hop_count = count;
        bytes(&request)
    };
    for count in [0, 1] {
        let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &hops(9, count), at(1));
        assert_eq!(outcome, Outcome::Discarded(Discard::HopCount));
        assert!(sent.is_empty(), "no reject either");
    }
    assert_eq!(router.counters().hop_count_drops, 2);
    assert_eq!(router.counters().rejects, 0);
    let spent = Event::HopCountSpent { network: 9 };
    assert_eq!(host.events, [spent, spent]);

    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &hops(9, 2), at(1));
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].to.as_deref(), Some(R7));
    assert_eq!(sent[0].parsed().hop_count, 1);

    let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &hops(2, 1), at(1));
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!((sent[0].port, sent[0].to.as_deref()), (1, Some(&[5][..])));
    assert_eq!(sent[0].parsed().destination, None);
    assert_eq!(router.counters().hop_count_drops, 2);
}

/// The same holds for network messages: one routed toward the next router
/// (an I-Am-Router for network 9, and a reject passed on toward a node
/// behind R7) that arrives with a hop count of 1 goes nowhere, counted, as
/// an APDU does. Each still does what it does at the router: the reject
/// marks 9 unreachable before it is passed on.
#[test]
fn hop_count_reaching_zero_stops_routed_network_messages() {
    let mut router = router();
    let mut host = Recorder::default();
    learn_9_via_r7(&mut router, at(0));
    let mut i_am = Npdu::message(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &[0, 44]);
    i_am.destination = Some(addr(9, &[5]));
    let data = fixture::reject_data(RejectMessageReason::NOT_DIRECTLY_CONNECTED, 9);
    let mut reject = Npdu::message(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK, &data);
    reject.destination = Some(addr(9, &[5]));
    for mut message in [i_am, reject] {
        message.hop_count = 1;
        let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &bytes(&message), at(1));
        assert_eq!(outcome, Outcome::Discarded(Discard::HopCount));
        assert!(sent.is_empty());
        message.hop_count = 2;
        let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &bytes(&message), at(1));
        assert_eq!((sent.len(), sent[0].to.as_deref()), (1, Some(R7)));
        assert_eq!(sent[0].parsed().hop_count, 1);
    }
    assert_eq!(router.counters().hop_count_drops, 2);
    assert!(router.table().lookup(44).is_none(), "routed, not learned");
}

/// Clauses 6.3.2 and 6.5.4: a global broadcast goes to every attached
/// network but the one it came from, keeping DNET 0xFFFF and DLEN 0, and is
/// delivered locally too. With its hop count spent it is only delivered:
/// one that arrives with 1 as well as 0.
#[test]
fn global_broadcast_fans_out_and_delivers() {
    let ports = [port_on(1, OWN_0), port_on(2, OWN_1), port_on(3, &[0x33])];
    let mut router = router_with(&ports, Options::HOSTED);
    let mut host = Recorder::default();
    let (outcome, sent) = deliver(
        &mut router,
        &mut host,
        1,
        &[0x20],
        &apdu_to(0xFFFF, &[], UNCONFIRMED),
        at(1),
    );
    assert!(matches!(
        outcome,
        Outcome::Local {
            group: true,
            global: true,
            ..
        }
    ));
    let egress: Vec<u8> = sent.iter().map(|sent| sent.port).collect();
    assert_eq!(egress, [0, 2]);
    for sent in &sent {
        assert!(sent.is_broadcast());
        assert_eq!(sent.kind, Kind::Global);
        let parsed = sent.parsed();
        assert_eq!(parsed.destination, Some(addr(0xFFFF, &[])));
        assert_eq!(parsed.source, Some(addr(2, &[0x20])));
        assert_eq!(parsed.hop_count, 254);
    }

    // Spent, with 1 or 0 left: delivered, not relayed, and the host hears
    // of it, as the hosted router logs it (router/forwarding.rs:117-120).
    let mut last = Npdu::apdu(UNCONFIRMED);
    last.destination = Some(addr(0xFFFF, &[]));
    for count in [1, 0] {
        last.hop_count = count;
        let (outcome, sent) = deliver(&mut router, &mut host, 1, &[0x20], &bytes(&last), at(1));
        assert!(matches!(outcome, Outcome::Local { global: true, .. }));
        assert!(sent.is_empty(), "{count}");
    }
    assert_eq!(router.counters().hop_count_drops, 2);
    let spent = Event::HopCountSpent { network: 0xFFFF };
    assert_eq!(host.events, [spent, spent]);
}

/// Clauses 6.2.5 and 6.5.4: a network message for another network, a
/// proprietary one included, is routed like an APDU, with its type and
/// vendor ID kept, and the router never takes it itself.
#[test]
fn proprietary_messages_route_opaquely() {
    let mut router = router();
    let mut host = Recorder::default();
    learn_9_via_r7(&mut router, at(0));
    let mut message = Npdu::message(NetworkMessageType::from_raw(0x80), &[0xDE, 0xAD]);
    message.vendor_id = Some(260);
    message.destination = Some(addr(9, &[5]));
    let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &bytes(&message), at(1));
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(sent[0].to.as_deref(), Some(R7));
    let parsed = sent[0].parsed();
    assert_eq!(
        (parsed.message_type, parsed.vendor_id),
        (Some(NetworkMessageType::from_raw(0x80)), Some(260))
    );
    assert_eq!(parsed.payload, [0xDE, 0xAD]);

    // An I-Am-Router for another network is routed too, not learned from.
    let mut i_am = Npdu::message(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &[0, 44]);
    i_am.destination = Some(addr(9, &[5]));
    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &bytes(&i_am), at(1));
    assert_eq!((sent.len(), sent[0].kind), (1, Kind::Forward));
    assert!(router.table().lookup(44).is_none());
    assert!(host.asked.is_empty());
}

/// What the writer can't write goes nowhere and is counted, as an encode
/// failure is logged and dropped today: an empty SADR from an empty link
/// source, and a global broadcast whose rewrite outgrows the scratch. A
/// port's limit is at most the scratch, so anything routed toward one
/// network meets that limit first.
#[test]
fn unwritable_rewrites_are_dropped_and_counted() {
    let mut router = router_with(&two_ports(), Options::HOSTED);
    let mut host = Recorder::default();
    let unwritable = Outcome::Discarded(Discard::Unwritable);
    let (outcome, _) = deliver(
        &mut router,
        &mut host,
        0,
        &[],
        &apdu_to(2, &[5], CONFIRMED),
        at(1),
    );
    assert_eq!(outcome, unwritable);
    // 1,497 octets in, 1,506 out: the source fields (3 + 6 octets) come
    // in, past the scratch. The router still takes its own copy.
    let mut long = Npdu::apdu(&[0x10; 1497 - 6]);
    long.destination = Some(addr(0xFFFF, &[]));
    let frame = bytes(&long);
    assert_eq!(frame.len(), 1497);
    let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &frame, at(1));
    assert!(matches!(outcome, Outcome::Local { global: true, .. }));
    assert!(sent.is_empty());
    assert_eq!(router.counters().write_failures, 2);
}

/// #1665 (Clauses 6.4.4 and 6.6.3.5, Table 6-1): an NPDU too long for
/// its egress port, as rewritten for it, draws reason 4 naming its DNET and
/// goes nowhere. That holds onto an attached network, toward the next
/// router, and for a remote broadcast, each by the limit for its kind of
/// send. A fan-out leaves the port out instead, counted, with no reject
/// (decision D21 on #1679).
#[test]
fn too_long_for_the_egress_port_draws_reason_4() {
    let mut ports = [port_on(1, OWN_0), port_on(2, OWN_1), port_on(3, &[0x33])];
    ports[1].max_npdu = NpduLimits {
        unicast: 600,
        broadcast: 501,
    };
    let mut router = router_with(&ports, Options::HOSTED);
    let mut host = Recorder::default();
    learn_9_via_r7(&mut router, at(0));

    // Onto network 2: 598 octets in grow by 4 to 602, two past port 1's
    // unicast limit, while 596 grow to 600 and fit.
    let to_2 = |len: usize| {
        let mut npdu = Npdu::apdu(&[0u8; 1497][..len - 7]);
        npdu.destination = Some(addr(2, &[5]));
        bytes(&npdu)
    };
    let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &to_2(598), at(1));
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(sent.len(), 1);
    assert_eq!((sent[0].port, sent[0].to.as_deref()), (0, Some(NODE_A)));
    assert_eq!(
        sent[0].kind,
        Kind::Reject(RejectMessageReason::MESSAGE_TOO_LONG)
    );
    assert_eq!(
        sent_reject(&sent[0]),
        (RejectMessageReason::MESSAGE_TOO_LONG, 2, None)
    );
    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &to_2(596), at(1));
    assert_eq!(sent.len(), 1);
    assert_eq!((sent[0].port, sent[0].npdu.len()), (1, 600));
    assert_eq!(sent[0].kind, Kind::Forward);

    // A remote broadcast onto network 2 goes by the broadcast limit: it
    // loses only the DNET, DLEN and hop count, so 497 octets in grow by 5
    // to 502, one past it.
    let remote = |len: usize| {
        let mut npdu = Npdu::apdu(&[0x10; 1497][..len - 6]);
        npdu.destination = Some(addr(2, &[]));
        bytes(&npdu)
    };
    let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &remote(497), at(1));
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent_reject(&sent[0]),
        (RejectMessageReason::MESSAGE_TOO_LONG, 2, None)
    );
    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &remote(496), at(1));
    assert_eq!((sent[0].port, sent[0].is_broadcast()), (1, true));
    assert_eq!(sent[0].npdu.len(), 501);

    // Toward R7 the destination fields and hop count stay and the source
    // fields come: 591 octets in, 600 out, fit; 592 draw reason 4 naming 9.
    let to_9 = |len: usize| {
        let mut npdu = Npdu::apdu(&[0u8; 1497][..len - 7]);
        npdu.destination = Some(addr(9, &[5]));
        bytes(&npdu)
    };
    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &to_9(591), at(1));
    assert_eq!((sent[0].to.as_deref(), sent[0].npdu.len()), (Some(R7), 600));
    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &to_9(592), at(1));
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent_reject(&sent[0]),
        (RejectMessageReason::MESSAGE_TOO_LONG, 9, None)
    );
    assert_eq!(router.counters().rejects, 3);
    assert_eq!(router.counters().egress_too_long, 3);

    // A global broadcast too long for port 1 still reaches port 2, and no
    // reject goes back.
    let mut global = Npdu::apdu(&[0x10; 600]);
    global.destination = Some(addr(0xFFFF, &[]));
    let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &bytes(&global), at(1));
    assert!(matches!(outcome, Outcome::Local { global: true, .. }));
    let ports: Vec<u8> = sent.iter().map(|sent| sent.port).collect();
    assert_eq!(ports, [2]);
    assert_eq!(router.counters().egress_too_long, 4);
    assert_eq!(router.counters().rejects, 3);
}

/// A fan-out goes by each port's broadcast limit, not its unicast one: a
/// global broadcast of 520 octets once rewritten skips port 1, which takes
/// 600 to one node but 501 to all, and reaches port 2, with no reject. A
/// unicast of the same length onto network 2 goes. The host hears of the
/// skip and of a reject alike.
#[test]
fn a_fan_out_goes_by_the_broadcast_limit() {
    let mut ports = [port_on(1, OWN_0), port_on(2, OWN_1), port_on(3, &[0x33])];
    ports[1].max_npdu = NpduLimits {
        unicast: 600,
        broadcast: 501,
    };
    let mut router = router_with(&ports, Options::HOSTED);
    let mut host = Recorder::default();
    // 511 octets in, plus SNET, SLEN and A's 6-octet SADR: 520.
    let mut global = Npdu::apdu(&[0x10; 505]);
    global.destination = Some(addr(0xFFFF, &[]));
    assert_eq!(bytes(&global).len(), 511);
    let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &bytes(&global), at(1));
    assert!(matches!(outcome, Outcome::Local { global: true, .. }));
    let egress: Vec<(u8, usize)> = sent.iter().map(|s| (s.port, s.npdu.len())).collect();
    assert_eq!(egress, [(2, 520)]);
    let skipped = Event::TooLong {
        port: PortId(1),
        octets: 520,
        limit: 501,
        rejected: None,
    };
    assert_eq!(host.events, [skipped]);
    assert_eq!(router.counters().egress_too_long, 1);
    assert_eq!(router.counters().rejects, 0);

    // 516 octets in grow by 4 to 520, within the unicast limit; 597 grow
    // to 601, past it, and draw reason 4.
    let to_2 = |len: usize| {
        let mut npdu = Npdu::apdu(&[0u8; 1497][..len - 7]);
        npdu.destination = Some(addr(2, &[5]));
        bytes(&npdu)
    };
    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &to_2(516), at(1));
    assert_eq!((sent[0].port, sent[0].npdu.len()), (1, 520));
    host.events.clear();
    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &to_2(597), at(1));
    assert_eq!(
        sent_reject(&sent[0]),
        (RejectMessageReason::MESSAGE_TOO_LONG, 2, None)
    );
    let refused = Event::TooLong {
        port: PortId(1),
        octets: 601,
        limit: 600,
        rejected: Some(2),
    };
    assert_eq!(host.events, [refused]);
}

/// The NPDU priority reaches every send, for a port that queues by it.
#[test]
fn sends_carry_the_npdu_priority() {
    let mut router = router();
    let mut host = Recorder::default();
    for priority in NetworkPriority::ALL {
        let mut request = Npdu::apdu(CONFIRMED);
        request.destination = Some(addr(2, &[5]));
        request.priority = priority;
        let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &bytes(&request), at(1));
        assert_eq!(sent[0].priority, priority);
    }
    assert_eq!(router.table().lookup(2).unwrap().port(), PortId(1));
}
