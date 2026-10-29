//! A node that doesn't route: what reaches its application and its
//! network-message consumer, what it drops and counts, and that its
//! admission checks are the router's (#1795).

mod fixture;

use bacnet_router_core::PortId;
use bacnet_router_core::node::{Apdu, Counters, Node, Received};
use bacnet_router_core::npdu::{AddressField, Malformed, NetworkMessageType, Npdu};
use bacnet_router_core::router::{Discard, Frame, Permissive, Received as RouterReceived};
use fixture::{CONFIRMED, NODE_A, UNCONFIRMED, addr, apdu_to, at, bytes, drain, message, router};
use proptest::prelude::*;

/// A node's counts, from zero, with `set` applied.
fn counts(set: impl FnOnce(&mut Counters)) -> Counters {
    let mut counters = Counters::default();
    set(&mut counters);
    counters
}

/// An APDU from `source` (SNET 9, SADR 5, when a router relayed it).
fn relayed(apdu: &[u8]) -> Vec<u8> {
    let mut npdu = Npdu::apdu(apdu);
    npdu.source = Some(addr(9, &[5]));
    bytes(&npdu)
}

/// Clause 6.5.2.1: an APDU with no DNET, or a global broadcast, reaches the
/// application, with its group and global flags and its relayed source.
#[test]
fn local_and_global_apdus_reach_the_application() {
    let mut node = Node::new();
    let local = bytes(&Npdu::apdu(CONFIRMED));
    let Received::Apdu(apdu) = node.receive(NODE_A, false, &local) else {
        panic!("an APDU");
    };
    assert_eq!(
        (apdu.npdu.payload, apdu.group, apdu.global),
        (CONFIRMED, false, false)
    );
    assert_eq!(apdu.npdu.payload_offset, 2);

    let link_broadcast = bytes(&Npdu::apdu(UNCONFIRMED));
    let Received::Apdu(apdu) = node.receive(NODE_A, true, &link_broadcast) else {
        panic!("an APDU");
    };
    assert_eq!((apdu.group, apdu.global), (true, false), "a link broadcast");

    let global = apdu_to(0xFFFF, &[], UNCONFIRMED);
    let Received::Apdu(Apdu {
        npdu,
        group,
        global,
        ..
    }) = node.receive(NODE_A, false, &global)
    else {
        panic!("an APDU");
    };
    assert_eq!((npdu.payload, group, global), (UNCONFIRMED, true, true));

    let relayed = relayed(CONFIRMED);
    let Received::Apdu(apdu) = node.receive(NODE_A, false, &relayed) else {
        panic!("an APDU");
    };
    assert_eq!(apdu.npdu.source, Some(addr(9, &[5])));
    assert_eq!(node.take_counters(), Counters::default());
}

/// Clause 6.5.2.2: a network message with no DNET, or a global one, goes to
/// the consumer.
#[test]
fn local_and_global_messages_reach_the_consumer() {
    let mut node = Node::new();
    let what_is = message(NetworkMessageType::WHAT_IS_NETWORK_NUMBER, &[]);
    let Received::Message(npdu) = node.receive(NODE_A, true, &what_is) else {
        panic!("a message");
    };
    assert_eq!(
        npdu.message_type,
        Some(NetworkMessageType::WHAT_IS_NETWORK_NUMBER)
    );

    let mut global = Npdu::message(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK, &[]);
    global.destination = Some(addr(0xFFFF, &[]));
    assert!(matches!(
        node.receive(NODE_A, false, &bytes(&global)),
        Received::Message(_)
    ));
    assert_eq!(node.counters(), Counters::default());
}

/// Clauses 6.5.2.1, 6.5.2.2 and 6.5.4: a DNET that names one network is for
/// a router to act on, so a node drops what carries one, APDU or network
/// message, unicast or remote broadcast.
#[test]
fn a_dnet_naming_one_network_is_dropped() {
    let mut node = Node::new();
    let mut message = Npdu::message(NetworkMessageType::WHAT_IS_NETWORK_NUMBER, &[]);
    message.destination = Some(addr(3, &[]));
    let frames = [
        apdu_to(3, &[5], CONFIRMED),
        apdu_to(3, &[], UNCONFIRMED),
        bytes(&message),
    ];
    for frame in &frames {
        assert_eq!(
            node.receive(NODE_A, false, frame),
            Received::Discarded(Discard::NotRouted)
        );
    }
    assert_eq!(
        node.take_counters(),
        counts(|c| c.routed_drops = frames.len() as u32)
    );
    assert_eq!(node.counters(), Counters::default(), "taking starts again");
}

/// The admission checks a router makes too: each refusal names its reason
/// and is counted under it.
#[test]
fn admission_refusals_are_named_and_counted() {
    let mut node = Node::new();
    let mut long_dadr = vec![1, 0x20, 0, 3, 19];
    long_dadr.extend_from_slice(&[0xAA; 19]);
    long_dadr.push(255);
    long_dadr.extend_from_slice(CONFIRMED);
    let cases: [(&[u8], Vec<u8>, Discard); 6] = [
        (
            &[1; 19],
            bytes(&Npdu::apdu(CONFIRMED)),
            Discard::SourceTooLong { length: 19 },
        ),
        (
            NODE_A,
            long_dadr,
            Discard::AddressTooLong {
                field: AddressField::Destination,
                length: 19,
            },
        ),
        (NODE_A, vec![1], Discard::Malformed(Malformed::Truncated)),
        (
            NODE_A,
            apdu_to(0xFFFF, &[5], UNCONFIRMED),
            Discard::GlobalWithDadr,
        ),
        (
            NODE_A,
            apdu_to(0xFFFF, &[], CONFIRMED),
            Discard::BroadcastNotUnconfirmed,
        ),
        (
            NODE_A,
            apdu_to(3, &[], CONFIRMED),
            Discard::BroadcastNotUnconfirmed,
        ),
    ];
    for (source, frame, discard) in &cases {
        assert_eq!(
            node.receive(source, false, frame),
            Received::Discarded(*discard),
            "{discard:?}"
        );
    }
    assert_eq!(
        node.take_counters(),
        counts(|c| {
            c.address_length_drops = 2;
            c.malformed = 1;
            c.global_broadcast_dadr_drops = 1;
            c.broadcast_pdu_type_drops = 2;
        })
    );
}

/// One frame for the agreement test: a source, short or too long, and an
/// NPDU, random or built to reach each check.
fn frame() -> impl Strategy<Value = (Vec<u8>, Vec<u8>)> {
    let source = prop_oneof![
        Just(NODE_A.to_vec()),
        proptest::collection::vec(any::<u8>(), 0..21),
    ];
    let npdu = prop_oneof![
        proptest::collection::vec(any::<u8>(), 0..40),
        (
            prop_oneof![Just(3u16), Just(0xFFFF)],
            proptest::collection::vec(any::<u8>(), 0..3),
            any::<bool>(),
        )
            .prop_map(|(dnet, dadr, unconfirmed)| {
                apdu_to(
                    dnet,
                    &dadr,
                    if unconfirmed { UNCONFIRMED } else { CONFIRMED },
                )
            }),
        (0u8..21, any::<bool>()).prop_map(|(length, destination)| {
            let mut out = vec![1, if destination { 0x20 } else { 0x08 }, 0, 3, length];
            out.extend(std::iter::repeat_n(0xAA, usize::from(length)));
            if destination {
                out.push(255);
            }
            out.extend_from_slice(CONFIRMED);
            out
        }),
    ];
    (source, npdu)
}

/// Whether a router or node discard is one of the shared admission checks.
fn admission(discard: Discard) -> bool {
    matches!(
        discard,
        Discard::SourceTooLong { .. }
            | Discard::AddressTooLong { .. }
            | Discard::Malformed(_)
            | Discard::GlobalWithDadr
            | Discard::BroadcastNotUnconfirmed
    )
}

proptest! {
    /// The node and the router refuse exactly the same frames at admission,
    /// for the same reasons, and count them under the same names.
    #[test]
    fn node_and_router_admit_alike(frames in proptest::collection::vec(frame(), 1..30)) {
        let mut router = router();
        let mut node = Node::new();
        for (index, (source, npdu)) in frames.iter().enumerate() {
            let routed = router.receive(
                Frame { port: PortId(0), source, link_group: false, npdu },
                at(index as u64),
                &mut Permissive,
            );
            drain(&mut router);
            let by_router = match routed {
                RouterReceived::Discarded(discard) if admission(discard) => Some(discard),
                _ => None,
            };
            let by_node = match node.receive(source, false, npdu) {
                Received::Discarded(discard) if admission(discard) => Some(discard),
                _ => None,
            };
            prop_assert_eq!(by_router, by_node);
        }
        let (routed, node) = (router.counters(), node.counters());
        prop_assert_eq!(routed.address_length_drops, node.address_length_drops);
        prop_assert_eq!(routed.malformed, node.malformed);
        prop_assert_eq!(routed.global_broadcast_dadr_drops, node.global_broadcast_dadr_drops);
        prop_assert_eq!(routed.broadcast_pdu_type_drops, node.broadcast_pdu_type_drops);
    }
}
