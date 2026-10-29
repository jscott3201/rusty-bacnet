//! A queued send's refusal uses its retained origin, not the current scratch.

mod fixture;

use bacnet_router_core::PortId;
use bacnet_router_core::npdu::Mac;
use bacnet_router_core::npdu::NetworkPriority;
use bacnet_router_core::npdu::{Address, Npdu};
use bacnet_router_core::npdu::{NetworkMessageType, RejectMessageReason};
use bacnet_router_core::router::{Action, Frame, Kind, Permissive, RejectOrigin, SendRefusal};
use fixture::{
    NODE_A, OWN_0, R7, TestRouter, UNCONFIRMED, addr, at, bytes, drain, learn_9_via_r7, message,
    router, sent_reject,
};

fn receive(router: &mut TestRouter, packet: &[u8]) {
    let _ = router.receive(
        Frame {
            port: PortId(0),
            source: NODE_A,
            link_group: false,
            npdu: packet,
        },
        at(1),
        &mut Permissive,
    );
}

fn origin(router: &mut TestRouter, packet: &[u8]) -> RejectOrigin {
    receive(router, packet);
    let Action::Send(send) = router.poll_action() else {
        panic!("routed send")
    };
    assert_eq!(send.kind, Kind::Forward);
    let origin = send.reject_origin.expect("eligible routed origin");
    assert_eq!(router.poll_action(), Action::Idle);
    origin
}

#[test]
fn late_refusal_keeps_original_dnet_and_local_routed_or_attached_return_path() {
    let remote = [0x42, 0x43];
    let attached = [0x55];
    let local = [0x66];
    for dnet in [2, 9] {
        for (source, port, to, destination) in [
            (None, 0, NODE_A, None),
            (
                Some(addr(70, &remote)),
                0,
                NODE_A,
                Some((70, remote.to_vec())),
            ),
            (Some(addr(2, &attached)), 1, &attached[..], None),
            (Some(addr(1, &local)), 0, &local[..], None),
        ] {
            let mut router = router();
            learn_9_via_r7(&mut router, at(0));
            let mut npdu = Npdu::apdu(UNCONFIRMED);
            npdu.destination = Some(addr(dnet, &[9]));
            npdu.source = source;
            let saved = origin(&mut router, &bytes(&npdu));
            // Both the incoming frame and later scratch contents may be gone.
            // The directly attached case lost its DNET on the queued NPDU.
            router.start();
            let _ = drain(&mut router);
            router.refused_later(SendRefusal::TooLong, Some(saved));
            let sent = drain(&mut router);
            assert_eq!(sent.len(), 1);
            assert_eq!((sent[0].port, sent[0].to.as_deref()), (port, Some(to)));
            assert_eq!(
                sent_reject(&sent[0]),
                (RejectMessageReason::MESSAGE_TOO_LONG, dnet, destination)
            );
            assert_eq!(sent[0].priority, NetworkPriority::Normal);
            let counts = router.counters();
            assert_eq!(
                (
                    counts.egress_too_long,
                    counts.refused,
                    counts.rejects,
                    counts.undrained
                ),
                (1, 1, 1, 0)
            );
        }
    }
}

#[test]
fn late_refusal_never_answers_forwarded_rejects_fanouts_or_own_origins() {
    let mut forwarded_reject =
        Npdu::message(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK, &[4, 0, 77]);
    forwarded_reject.destination = Some(addr(2, &[9]));
    let mut global = Npdu::apdu(UNCONFIRMED);
    global.destination = Some(addr(0xffff, &[]));
    let control = Npdu::message(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &[0, 9]);
    let mut own_origin = Npdu::apdu(UNCONFIRMED);
    own_origin.destination = Some(addr(2, &[9]));
    own_origin.source = Some(addr(1, OWN_0));
    for (npdu, expected) in [
        (forwarded_reject, Kind::Forward),
        (global, Kind::Global),
        (
            control,
            Kind::Relay(NetworkMessageType::I_AM_ROUTER_TO_NETWORK),
        ),
        (own_origin, Kind::Forward),
    ] {
        let mut router = router();
        receive(&mut router, &bytes(&npdu));
        let Action::Send(send) = router.poll_action() else {
            panic!("forward or relay")
        };
        assert_eq!(send.kind, expected);
        assert_eq!(send.reject_origin, None);
        assert_eq!(router.poll_action(), Action::Idle);
        router.refused_later(SendRefusal::TooLong, None);
        assert_eq!(router.poll_action(), Action::Idle);
        let counts = router.counters();
        assert_eq!(
            (counts.egress_too_long, counts.refused, counts.rejects),
            (1, 1, 0)
        );
    }
}

#[test]
fn a_generated_rejects_refusal_and_other_causes_do_not_make_feedback() {
    for why in [SendRefusal::Full, SendRefusal::PortDown, SendRefusal::Other] {
        let mut router = router();
        let mut npdu = Npdu::apdu(UNCONFIRMED);
        npdu.destination = Some(addr(2, &[9]));
        let saved = origin(&mut router, &bytes(&npdu));
        router.refused_later(why, Some(saved));
        assert_eq!(router.poll_action(), Action::Idle);
        assert_eq!(router.counters().refused, 1);
        assert_eq!(router.counters().egress_too_long, 0);
        assert_eq!(router.counters().rejects, 0);
    }
    let mut router = router();
    let mut npdu = Npdu::apdu(UNCONFIRMED);
    npdu.destination = Some(addr(2, &[9]));
    let saved = origin(&mut router, &bytes(&npdu));
    router.refused_later(SendRefusal::TooLong, Some(saved));
    let Action::Send(send) = router.poll_action() else {
        panic!("reason 4")
    };
    assert_eq!(send.reject_origin, None);
    assert_eq!(
        send.kind,
        Kind::Reject(RejectMessageReason::MESSAGE_TOO_LONG)
    );
    router.refused_later(SendRefusal::TooLong, None);
    assert_eq!(router.poll_action(), Action::Idle);
    let counts = router.counters();
    assert_eq!(
        (counts.egress_too_long, counts.refused, counts.rejects),
        (2, 2, 1)
    );
}

#[test]
fn a_late_reject_follows_a_waiting_table_answer_without_ending_it() {
    let mut router = router();
    let mut npdu = Npdu::apdu(UNCONFIRMED);
    npdu.destination = Some(Address {
        network: 2,
        mac: &[9],
    });
    let saved = origin(&mut router, &bytes(&npdu));
    for network in 100..230 {
        router
            .table_mut()
            .add_learned(network, PortId(1), Mac::from_slice(R7).unwrap(), at(0));
    }
    receive(
        &mut router,
        &message(NetworkMessageType::INITIALIZE_ROUTING_TABLE, &[0]),
    );
    let Action::Send(first) = router.poll_action() else {
        panic!("first Ack")
    };
    assert_eq!(
        first.kind,
        Kind::Answer(NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK)
    );
    router.refused_later(SendRefusal::TooLong, Some(saved));
    let sent = drain(&mut router);
    assert_eq!(sent.len(), 2);
    assert_eq!(
        sent[0].kind,
        Kind::Answer(NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK)
    );
    assert_eq!(
        sent[1].kind,
        Kind::Reject(RejectMessageReason::MESSAGE_TOO_LONG)
    );
    assert_eq!(router.counters().undrained, 0);
}
