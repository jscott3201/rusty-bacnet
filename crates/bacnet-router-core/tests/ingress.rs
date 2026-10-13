//! A frame's way in: the address bounds, the drop rules, and local
//! delivery, in the hosted router's order (router/dispatch.rs:80-207).

mod fixture;

use bacnet_router_core::message::{MessageType, RejectReason};
use bacnet_router_core::npdu::{AddressField, Malformed, Npdu};
use bacnet_router_core::router::{Discard, Kind, Options};
use fixture::{
    addr, apdu_to, at, bytes, deliver, deliver_broadcast, learn_9_via_r7, port_on, router,
    router_counts, router_with, sent_reject, Outcome, Recorder, CONFIRMED, NODE_A, OWN_0, OWN_1,
    R7, UNCONFIRMED,
};

/// An NPDU with a DLEN or SLEN of 19, one past the bound, for DNET `dnet`
/// (or none), with a valid SNET and SADR behind the DADR when `source`.
fn over_long(dnet: Option<u16>, long_source: bool, source: Option<(u16, &[u8])>) -> Vec<u8> {
    let mut control = 0u8;
    let mut out = vec![1];
    let mut body = Vec::new();
    if let Some(dnet) = dnet {
        control |= 0x20;
        body.extend_from_slice(&dnet.to_be_bytes());
        if long_source {
            body.extend_from_slice(&[1, 0x05]);
        } else {
            body.push(19);
            body.extend_from_slice(&[0xAA; 19]);
        }
    }
    if long_source {
        control |= 0x08;
        body.extend_from_slice(&[0, 7, 19]);
        body.extend_from_slice(&[0xBB; 19]);
    } else if let Some((network, mac)) = source {
        control |= 0x08;
        body.extend_from_slice(&network.to_be_bytes());
        body.push(mac.len() as u8);
        body.extend_from_slice(mac);
    }
    if dnet.is_some() {
        body.push(255);
    }
    body.extend_from_slice(CONFIRMED);
    out.push(control);
    out.extend(body);
    out
}

/// Clause 6.2.2.2 and 6.4.4: an address length no link uses draws reason 6
/// when the NPDU names a network, sent where any reject goes; a global
/// broadcast, or an NPDU with no DNET, is only dropped
/// (router/reject.rs:187-211).
#[test]
fn over_long_addresses_reject_with_reason_6() {
    let mut router = router();
    let mut host = Recorder::default();
    let dropped = |field, length| Outcome::Discarded(Discard::AddressTooLong { field, length });

    // A long DADR from a local sender: reason 6 back to it, naming 9.
    let (outcome, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &over_long(Some(9), false, None),
        at(1),
    );
    assert_eq!(outcome, dropped(AddressField::Destination, 19));
    assert_eq!(sent.len(), 1);
    assert_eq!((sent[0].port, sent[0].to.as_deref()), (0, Some(NODE_A)));
    assert_eq!(sent[0].kind, Kind::Reject(RejectReason::ADDRESSING_ERROR));
    assert_eq!(
        sent_reject(&sent[0]),
        (RejectReason::ADDRESSING_ERROR, 9, None)
    );

    // A long DADR with a remote origin behind it: the reject goes to the
    // origin, through the router that relayed the NPDU.
    let frame = over_long(Some(9), false, Some((7, &[1, 2, 3])));
    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &frame, at(1));
    assert_eq!(sent[0].to.as_deref(), Some(NODE_A));
    assert_eq!(
        sent_reject(&sent[0]),
        (RejectReason::ADDRESSING_ERROR, 9, Some((7, vec![1, 2, 3])))
    );

    // A long SADR: the origin can't be named, so the link sender hears.
    let (outcome, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &over_long(Some(9), true, None),
        at(1),
    );
    assert_eq!(outcome, dropped(AddressField::Source, 19));
    assert_eq!(
        sent_reject(&sent[0]),
        (RejectReason::ADDRESSING_ERROR, 9, None)
    );

    // A global broadcast, and an NPDU with no DNET: dropped, unanswered.
    let (outcome, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &over_long(Some(0xFFFF), false, None),
        at(1),
    );
    assert_eq!(outcome, dropped(AddressField::Destination, 19));
    assert!(sent.is_empty());
    let (outcome, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &over_long(None, true, None),
        at(1),
    );
    assert_eq!(outcome, dropped(AddressField::Source, 19));
    assert!(sent.is_empty());

    assert_eq!(router.counters().address_length_drops, 5);
    assert_eq!(router.counters().rejects, 3);
}

/// #1198: a link source past the bound is dropped before the NPDU is read,
/// and nothing is sent back to it.
#[test]
fn an_over_long_link_source_is_dropped_unanswered() {
    let mut router = router();
    let mut host = Recorder::default();
    let source = [7u8; 19];
    let (outcome, sent) = deliver(
        &mut router,
        &mut host,
        0,
        &source,
        &apdu_to(9, &[5], CONFIRMED),
        at(1),
    );
    assert_eq!(
        outcome,
        Outcome::Discarded(Discard::SourceTooLong { length: 19 })
    );
    assert!(sent.is_empty());
    assert_eq!(router.counters().address_length_drops, 1);
    // Eighteen octets is within the bound.
    let (outcome, _) = deliver(
        &mut router,
        &mut host,
        0,
        &[7; 18],
        &apdu_to(9, &[5], CONFIRMED),
        at(1),
    );
    assert_eq!(outcome, Outcome::Handled);
}

#[test]
fn malformed_frames_and_unknown_ports_are_dropped_and_counted() {
    let mut router = router();
    let mut host = Recorder::default();
    let (outcome, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &[2, 0, 0x10, 0x08],
        at(1),
    );
    assert_eq!(
        outcome,
        Outcome::Discarded(Discard::Malformed(Malformed::Version))
    );
    assert!(sent.is_empty());
    let (outcome, _) = deliver(&mut router, &mut host, 0, NODE_A, &[1], at(1));
    assert_eq!(
        outcome,
        Outcome::Discarded(Discard::Malformed(Malformed::Truncated))
    );
    let (outcome, _) = deliver(
        &mut router,
        &mut host,
        2,
        NODE_A,
        &apdu_to(9, &[5], CONFIRMED),
        at(1),
    );
    assert_eq!(outcome, Outcome::Discarded(Discard::UnknownPort));
    assert_eq!(
        router.counters(),
        router_counts(|c| {
            c.malformed = 2;
            c.unknown_port = 1;
        })
    );
}

/// #1379: a DADR beside DNET 0xFFFF contradicts it. Dropped before
/// anything else looks at the NPDU: not relayed, delivered, answered or
/// learned from, network messages included.
#[test]
fn a_global_broadcast_with_a_dadr_goes_nowhere() {
    let mut router = router();
    let mut host = Recorder::default();
    for frame in [apdu_to(0xFFFF, &[5], UNCONFIRMED), {
        let mut npdu = Npdu::message(MessageType::I_AM_ROUTER_TO_NETWORK, &[0, 9]);
        npdu.destination = Some(addr(0xFFFF, &[5]));
        bytes(&npdu)
    }] {
        let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &frame, at(1));
        assert_eq!(outcome, Outcome::Discarded(Discard::GlobalWithDadr));
        assert!(sent.is_empty());
    }
    assert!(router.table().lookup(9).is_none());
    assert!(host.asked.is_empty());
    assert_eq!(router.counters().global_broadcast_dadr_drops, 2);
}

/// Clause 6.3 (#1491): only an Unconfirmed-Request may go to a broadcast
/// network address, global or remote. Network messages are not affected.
#[test]
fn broadcast_addresses_carry_only_unconfirmed() {
    let mut router = router();
    let mut host = Recorder::default();
    let refused = Outcome::Discarded(Discard::BroadcastNotUnconfirmed);
    for frame in [
        apdu_to(0xFFFF, &[], CONFIRMED),
        apdu_to(2, &[], CONFIRMED),
        apdu_to(0xFFFF, &[], &[]),
        apdu_to(2, &[], &[0x30, 0x01]),
    ] {
        let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &frame, at(1));
        assert_eq!(outcome, refused);
        assert!(sent.is_empty(), "no reject either");
    }
    assert_eq!(router.counters().broadcast_pdu_type_drops, 4);

    let (outcome, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &apdu_to(2, &[], UNCONFIRMED),
        at(1),
    );
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(sent.len(), 1);
    let mut who_is = Npdu::message(MessageType::WHO_IS_ROUTER_TO_NETWORK, &[]);
    who_is.destination = Some(addr(0xFFFF, &[]));
    let (outcome, _) = deliver(&mut router, &mut host, 0, NODE_A, &bytes(&who_is), at(1));
    assert_eq!(outcome, Outcome::Handled);
}

/// Local delivery: what each kind of address means for the router's own
/// application (router/dispatch.rs:137-194; layer.rs:301-306).
#[test]
fn local_delivery_reports_how_it_was_addressed() {
    let mut router = router();
    let mut host = Recorder::default();
    let local = |apdu: &[u8], offset, group, global| Outcome::Local {
        apdu: apdu.to_vec(),
        apdu_offset: offset,
        source: None,
        group,
        global,
        expecting_reply: false,
        priority: 0,
    };

    // No DNET: the link says whether it was a group delivery.
    let plain = bytes(&Npdu::apdu(CONFIRMED));
    let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &plain, at(1));
    assert_eq!(outcome, local(CONFIRMED, 2, false, false));
    assert!(sent.is_empty());
    let (outcome, _) = deliver_broadcast(&mut router, &mut host, 0, NODE_A, &plain, at(1));
    assert_eq!(outcome, local(CONFIRMED, 2, true, false));

    // The router's own MAC on the arrival network: delivered, no relay.
    let (outcome, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &apdu_to(1, OWN_0, CONFIRMED),
        at(1),
    );
    assert_eq!(outcome, local(CONFIRMED, 12, false, false));
    assert!(sent.is_empty());

    // A global broadcast: delivered as a group, and passed on.
    let (outcome, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &apdu_to(0xFFFF, &[], UNCONFIRMED),
        at(1),
    );
    assert_eq!(outcome, local(UNCONFIRMED, 6, true, true));
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].kind, Kind::Global);

    // SNET, SADR, priority and expecting-reply reach the application.
    let mut routed = Npdu::apdu(CONFIRMED);
    routed.source = Some(addr(7, &[1, 2]));
    routed.priority = 2;
    routed.expecting_reply = true;
    let (outcome, _) = deliver(&mut router, &mut host, 1, R7, &bytes(&routed), at(1));
    assert_eq!(
        outcome,
        Outcome::Local {
            apdu: CONFIRMED.to_vec(),
            apdu_offset: 7,
            source: Some((7, vec![1, 2])),
            group: false,
            global: false,
            expecting_reply: true,
            priority: 2,
        }
    );
}

/// The arrival network as DNET: a DLEN 0 broadcast is delivered and also
/// sent back out the arrival port, and any other DADR goes back out the
/// arrival port alone (the "hairpin", pinned by group_dadr_tests.rs:96).
#[test]
fn the_arrival_network_as_dnet_goes_back_out_the_arrival_port() {
    let mut router = router();
    let mut host = Recorder::default();

    let (outcome, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &apdu_to(1, &[], UNCONFIRMED),
        at(1),
    );
    assert!(matches!(
        outcome,
        Outcome::Local {
            group: true,
            global: false,
            ..
        }
    ));
    assert_eq!(sent.len(), 1);
    assert_eq!((sent[0].port, sent[0].is_broadcast()), (0, true));
    let parsed = sent[0].parsed();
    assert_eq!(parsed.destination, None);
    assert_eq!(parsed.source, Some(addr(1, NODE_A)));

    const OTHER: &[u8] = &[10, 0, 0, 44, 0xBA, 0xC0];
    let (outcome, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &apdu_to(1, OTHER, CONFIRMED),
        at(1),
    );
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!((sent[0].port, sent[0].to.as_deref()), (0, Some(OTHER)));

    // With its hop count spent, the DLEN 0 broadcast is still delivered,
    // and the relay is dropped and counted.
    let mut spent = Npdu::apdu(UNCONFIRMED);
    spent.destination = Some(addr(1, &[]));
    spent.hop_count = 0;
    let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &bytes(&spent), at(1));
    assert!(matches!(outcome, Outcome::Local { group: true, .. }));
    assert!(sent.is_empty());
    assert_eq!(router.counters().hop_count_drops, 1);
}

/// Clause 6.1, Figure 6-12: an APDU for the router's own address on any of
/// its networks is the router's own application's, as one for its address
/// on the arrival network is. It is delivered with its
/// origin, as a routed APDU, and nothing goes on any wire.
#[test]
fn own_address_on_any_port_is_local() {
    let mut router = router();
    let mut host = Recorder::default();
    let mut request = Npdu::apdu(CONFIRMED);
    request.destination = Some(addr(2, OWN_1));
    request.expecting_reply = true;
    let frame = bytes(&request);
    let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &frame, at(1));
    assert_eq!(
        outcome,
        Outcome::Local {
            apdu: CONFIRMED.to_vec(),
            apdu_offset: frame.len() - CONFIRMED.len(),
            source: None,
            group: false,
            global: false,
            expecting_reply: true,
            priority: 0,
        }
    );
    assert!(sent.is_empty(), "nothing on any wire");
    assert_eq!(router.counters(), router_counts(|_| {}));

    // From another network through a router: the same, with the origin.
    let mut routed = Npdu::apdu(CONFIRMED);
    routed.destination = Some(addr(1, OWN_0));
    routed.source = Some(addr(7, &[3]));
    let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &bytes(&routed), at(1));
    assert!(matches!(
        outcome,
        Outcome::Local {
            source: Some((7, ref mac)),
            group: false,
            ..
        } if mac == &[3]
    ));
    assert!(sent.is_empty());
}

/// The router's own address is matched whole, network and MAC: a DADR
/// that equals one of its own MACs on a network behind another router
/// names some other node, so it is routed on. That is the usual MS/TP case,
/// where the router is station 1 on its link and another station 1 sits on
/// network 9, behind R7.
#[test]
fn own_mac_on_a_remote_network_is_routed_on() {
    let mut router = router();
    let mut host = Recorder::default();
    learn_9_via_r7(&mut router, at(0));
    for mac in [OWN_1, OWN_0] {
        let frame = apdu_to(9, mac, CONFIRMED);
        let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &frame, at(1));
        assert_eq!(outcome, Outcome::Handled);
        assert_eq!((sent.len(), sent[0].to.as_deref()), (1, Some(R7)));
        assert_eq!(sent[0].parsed().destination, Some(addr(9, mac)));
    }
}

/// A DLEN 0 broadcast is never the router's own address, even on a port
/// whose own MAC is empty, which a DLEN 0 would match: it goes out as a
/// broadcast.
#[test]
fn a_dlen_0_broadcast_is_never_the_routers_own_address() {
    let ports = [port_on(1, OWN_0), port_on(2, &[])];
    let mut router = router_with(&ports, Options::HOSTED);
    let mut host = Recorder::default();
    let frame = apdu_to(2, &[], UNCONFIRMED);
    let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &frame, at(1));
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!((sent[0].port, sent[0].is_broadcast()), (1, true));
}

/// A port numbered 0 or 0xFFFF is taken with no direct route, as today.
#[test]
fn a_reserved_port_number_has_no_direct_route() {
    let router = router_with(&[port_on(0, OWN_0), port_on(2, OWN_1)], Options::HOSTED);
    assert_eq!(router.table().networks().collect::<Vec<_>>(), [2]);
}
