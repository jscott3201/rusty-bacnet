//! The rejects the router sends, and the ones it passes on
//! (router/reject.rs).

mod fixture;

use bacnet_router_core::npdu::Npdu;
use bacnet_router_core::npdu::{NetworkMessageType, RejectMessageReason};
use bacnet_router_core::router::{ControlClass, Discard, Event, Kind};
use bacnet_router_core::table::{Reachability, RejectEffect};
use fixture::{
    addr, apdu_to, at, bytes, deliver, learn_9_via_r7, reject_data, router, sent_reject, Outcome,
    Recorder, CONFIRMED, NODE_A, OWN_1, R7,
};

/// A request for unknown network 44, with `source` as its SNET and SADR.
fn unknown_from(source: Option<(u16, &[u8])>) -> Vec<u8> {
    let mut request = Npdu::apdu(CONFIRMED);
    request.destination = Some(addr(44, &[5]));
    request.source = source.map(|(network, mac)| addr(network, mac));
    bytes(&request)
}

/// Clauses 6.4.4 and 6.5.4: a reject goes to whoever first sent the
/// refused NPDU, as its SNET and SADR name them (router/reject.rs:82-93).
#[test]
fn reject_path_follows_the_origin() {
    let mut router = router();
    let mut host = Recorder::default();
    let reject_of = |sent: &[fixture::Sent]| {
        sent.iter()
            .find(|sent| matches!(sent.kind, Kind::Reject(_)))
            .cloned()
    };

    // No SNET: the link sender, on the arrival port.
    let (_, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &unknown_from(None),
        at(1),
    );
    let reject = reject_of(&sent).unwrap();
    assert_eq!((reject.port, reject.to.as_deref()), (0, Some(NODE_A)));
    assert_eq!(sent_reject(&reject).2, None);

    // An SNET that is one of the router's networks: the SADR, on that
    // network's port, with no DNET, even though the NPDU came in on port 0.
    let (_, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &unknown_from(Some((2, &[0x09]))),
        at(10),
    );
    let reject = reject_of(&sent).unwrap();
    assert_eq!((reject.port, reject.to.as_deref()), (1, Some(&[0x09][..])));
    assert_eq!(sent_reject(&reject).2, None);

    // The router itself: nobody.
    let (outcome, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &unknown_from(Some((2, OWN_1))),
        at(20),
    );
    assert_eq!(outcome, Outcome::Handled);
    assert!(reject_of(&sent).is_none());
    assert_eq!(host.events, [Event::OwnOrigin { network: 44 }]);

    // A remote SNET whose SADR happens to equal one of the router's MACs:
    // another node, so it is answered, through the link sender.
    let (_, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &unknown_from(Some((7, OWN_1))),
        at(25),
    );
    let reject = reject_of(&sent).unwrap();
    assert_eq!(
        sent_reject(&reject),
        (
            RejectMessageReason::NOT_DIRECTLY_CONNECTED,
            44,
            Some((7, OWN_1.to_vec()))
        )
    );

    // A remote SNET: back to the link sender, naming the origin.
    let (_, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &unknown_from(Some((7, &[1, 2, 3]))),
        at(30),
    );
    let reject = reject_of(&sent).unwrap();
    assert_eq!((reject.port, reject.to.as_deref()), (0, Some(NODE_A)));
    assert_eq!(
        sent_reject(&reject),
        (
            RejectMessageReason::NOT_DIRECTLY_CONNECTED,
            44,
            Some((7, vec![1, 2, 3]))
        )
    );
    assert_eq!(router.counters().rejects, 4);
}

/// Clause 6.6.3.5: a received reject updates the table, then goes on toward
/// the node its DNET and DADR name, routed like anything else; one for the
/// router stops here (router/reject.rs:228-256).
#[test]
fn received_rejects_relay_by_their_destination() {
    let mut router = router();
    let mut host = Recorder::default();
    learn_9_via_r7(&mut router, at(0));
    let reject = |dnet: Option<(u16, &[u8])>| {
        let data = reject_data(RejectMessageReason::NOT_DIRECTLY_CONNECTED, 9);
        let mut npdu = Npdu::message(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK, &data);
        npdu.destination = dnet.map(|(network, mac)| addr(network, mac));
        bytes(&npdu)
    };

    // For A, on network 1: R7's reject marks 9, and reaches A with SNET 2
    // and R7 as its SADR.
    let (outcome, sent) = deliver(
        &mut router,
        &mut host,
        1,
        R7,
        &reject(Some((1, NODE_A))),
        at(1),
    );
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(
        router.table().lookup(9).unwrap().reachability(),
        Reachability::Unreachable
    );
    assert_eq!((sent[0].port, sent[0].to.as_deref()), (0, Some(NODE_A)));
    assert_eq!(sent[0].kind, Kind::Forward);
    let parsed = sent[0].parsed();
    assert_eq!(
        (parsed.destination, parsed.source),
        (None, Some(addr(2, R7)))
    );
    assert_eq!(
        parsed.message_type,
        Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK)
    );

    // For a node behind R7: passed to R7 though 9 is unreachable, since a
    // reject is routed without a reachability check.
    let (_, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &reject(Some((9, &[5]))),
        at(2),
    );
    assert_eq!((sent[0].port, sent[0].to.as_deref()), (1, Some(R7)));
    assert_eq!(sent[0].parsed().hop_count, 254);

    // For the router: no DNET, or its own address on any port.
    for (frame, port, source) in [(reject(None), 1, R7), (reject(Some((2, OWN_1))), 0, NODE_A)] {
        let (outcome, sent) = deliver(&mut router, &mut host, port, source, &frame, at(3));
        assert_eq!(outcome, Outcome::ToRouter(frame));
        assert!(sent.is_empty());
    }

    // Nowhere to go: 0xFFFF, the arrival network, a network with no route.
    for frame in [
        reject(Some((0xFFFF, &[]))),
        reject(Some((2, &[0x42]))),
        reject(Some((44, &[5]))),
    ] {
        let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &frame, at(4));
        assert_eq!(outcome, Outcome::Discarded(Discard::Unroutable));
        assert!(sent.is_empty(), "nothing answers a reject");
    }
    assert_eq!(router.counters().unroutable_rejects, 3);
    assert_eq!(router.counters().rejects, 0);
}

/// Clause 6.6.3.5: a reject with a reason other than 1 or 2, such as the
/// reason 4 that the egress limits make routine, leaves the route to the
/// network it names alone, and is still passed on toward its DNET, or kept
/// for the router itself.
#[test]
fn other_reasons_relay_and_keep_the_route() {
    let mut router = router();
    let mut host = Recorder::default();
    learn_9_via_r7(&mut router, at(0));
    let data = reject_data(RejectMessageReason::MESSAGE_TOO_LONG, 9);
    let mut reject = Npdu::message(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK, &data);
    reject.destination = Some(addr(1, NODE_A));
    let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &bytes(&reject), at(1));
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(sent.len(), 1, "passed on");
    assert_eq!((sent[0].port, sent[0].to.as_deref()), (0, Some(NODE_A)));
    assert_eq!(sent[0].parsed().payload, [4, 0, 9]);
    assert_eq!(
        router.table().lookup(9).map(|route| route.reachability()),
        Some(Reachability::Reachable)
    );
    assert_eq!(
        host.events,
        [Event::RejectReceived {
            network: 9,
            reason: RejectMessageReason::MESSAGE_TOO_LONG,
            effect: RejectEffect::Kept,
        }]
    );
}

/// RB-09: a refused reject changes nothing and goes nowhere; a malformed
/// one is refused before the host is asked.
#[test]
fn denied_and_malformed_rejects_do_nothing() {
    let mut router = router();
    let mut host = Recorder {
        deny: vec![ControlClass::Reject],
        ..Recorder::default()
    };
    learn_9_via_r7(&mut router, at(0));
    let data = reject_data(RejectMessageReason::NOT_DIRECTLY_CONNECTED, 9);
    let mut npdu = Npdu::message(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK, &data);
    npdu.destination = Some(addr(1, NODE_A));
    let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &bytes(&npdu), at(1));
    assert_eq!(outcome, Outcome::Discarded(Discard::Denied));
    assert!(sent.is_empty());
    assert_eq!(
        router.table().lookup(9).unwrap().reachability(),
        Reachability::Reachable
    );
    assert_eq!(host.asked[0].targets, [9]);
    assert_eq!(host.asked[0].dnet, Some(1));

    let short = fixture::message(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK, &[1, 0]);
    let (outcome, _) = deliver(&mut router, &mut host, 1, R7, &short, at(1));
    assert_eq!(outcome, Outcome::Discarded(Discard::BadMessage));
    assert_eq!(host.asked.len(), 1);
    assert_eq!(router.counters().denied, 1);
}

/// A reject padded past its three octets is refused whole, before the host
/// is asked: the route stays, nothing is sent, and it counts as malformed.
#[test]
fn a_padded_reject_is_a_bad_message() {
    let mut router = router();
    let mut host = Recorder::default();
    learn_9_via_r7(&mut router, at(0));
    let before = router.counters().bad_messages;
    let padded = fixture::message(
        NetworkMessageType::REJECT_MESSAGE_TO_NETWORK,
        &[
            RejectMessageReason::NOT_DIRECTLY_CONNECTED.to_raw(),
            0,
            9,
            0,
        ],
    );
    let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &padded, at(1));
    assert_eq!(outcome, Outcome::Discarded(Discard::BadMessage));
    assert!(sent.is_empty());
    assert!(host.asked.is_empty());
    assert_eq!(router.counters().bad_messages, before + 1);
    assert_eq!(
        router.table().lookup(9).unwrap().reachability(),
        Reachability::Reachable
    );
}

/// A reject the router sends for an unknown DNET also starts a search, and
/// an unreachable or busy route draws reason 1 or 2, with no search.
#[test]
fn unusable_routes_draw_their_reasons() {
    let mut router = router();
    let mut host = Recorder::default();
    learn_9_via_r7(&mut router, at(0));
    router.table_mut().mark_unreachable(9);
    let (_, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &apdu_to(9, &[5], CONFIRMED),
        at(1),
    );
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent_reject(&sent[0]).0,
        RejectMessageReason::NOT_DIRECTLY_CONNECTED
    );
    // The lookup still counts as use, as the hosted lookup touches first.
    assert_eq!(router.table().lookup(9).unwrap().last_used(), Some(at(1)));

    router.table_mut().mark_available(9);
    router.table_mut().mark_busy(9, at(30));
    let (_, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &apdu_to(9, &[5], CONFIRMED),
        at(2),
    );
    assert_eq!(sent_reject(&sent[0]).0, RejectMessageReason::ROUTER_BUSY);
    assert_eq!(router.counters().solicitations, 0);
}
