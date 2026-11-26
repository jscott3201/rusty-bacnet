//! What a node takes from inside a Forwarded-NPDU (#2829).
//!
//! A Forwarded-NPDU carries a broadcast another BBMD distributes (Annex
//! J.2.5, J.4.5), and its receiver takes the origin it names as the NPDU's
//! source. Nothing checks that origin: any station that can reach the node
//! can send one naming any address. A broadcast carries only an
//! Unconfirmed-Request or a network message (Clause 6.3), and a directed
//! message travels in an Original-Unicast-NPDU (J.3), so an answer or a
//! confirmed request inside one is never real traffic from its origin: the
//! node drops it rather than show it as that origin's.

mod node_support;

use core::net::SocketAddrV4;

use bacnet_bip_codec::Bvll;
use bacnet_bip_core::node::{
    Addressed, Discard, ForBbmd, ForwardedTrust, Link, Mode, Received, forward_content_allowed,
};
use node_support::*;

/// NPDU headers: version 1, then the control octet and its addresses.
const LOCAL: &[u8] = &[0x01, 0x00];
/// SNET 5, SADR 7: an answer a router relays from a device on network 5.
const FROM_NETWORK_5: &[u8] = &[0x01, 0x08, 0x00, 0x05, 0x01, 0x07];
/// DNET 5, DADR 7, hop count 255: for a router to pass on.
const TO_NETWORK_5: &[u8] = &[0x01, 0x20, 0x00, 0x05, 0x01, 0x07, 0xFF];
/// DNET 65535: a global broadcast.
const GLOBAL: &[u8] = &[0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF];

/// APDUs no broadcast carries: the five answers to a confirmed request, a
/// server's Segment-ACK and Abort, and a confirmed request.
const DIRECTED: [(&str, &[u8]); 8] = [
    ("SimpleACK", &[0x20, 0x05, 0x0F]),
    (
        "ComplexACK",
        &[0x30, 0x05, 0x0C, 0x0C, 0x02, 0x00, 0x00, 0x07],
    ),
    (
        "segmented ComplexACK",
        &[0x3C, 0x05, 0x00, 0x01, 0x0C, 0xAA],
    ),
    ("Segment-ACK", &[0x41, 0x05, 0x00, 0x01]),
    ("Error", &[0x50, 0x05, 0x0F, 0x91, 0x02, 0x91, 0x28]),
    ("Reject", &[0x60, 0x05, 0x00]),
    ("Abort", &[0x71, 0x05, 0x00]),
    (
        "ConfirmedRequest",
        &[0x00, 0x05, 0x05, 0x0C, 0x0C, 0x02, 0x00, 0x00, 0x07],
    ),
];

/// A Who-Is and an I-Am: what a BBMD forwards every day.
const UNCONFIRMED: [(&str, &[u8]); 2] = [
    ("Who-Is", &[0x10, 0x08]),
    (
        "I-Am",
        &[
            0x10, 0x00, 0xC4, 0x02, 0x00, 0x00, 0x07, 0x22, 0x01, 0xE0, 0x91, 0x00, 0x21, 0x07,
        ],
    ),
];

fn npdu(header: &[u8], apdu: &[u8]) -> Vec<u8> {
    [header, apdu].concat()
}

fn forwarded(npdu: &[u8]) -> Vec<u8> {
    wire(Bvll::ForwardedNpdu {
        origin: REMOTE,
        npdu,
    })
}

/// The source `link` hands `npdu` up as, inside a Forwarded-NPDU from
/// `sender` sent as `to` says, or `None` when it doesn't hand it up.
fn delivered_from(
    link: &Link,
    npdu: &[u8],
    sender: SocketAddrV4,
    to: Addressed,
) -> Option<SocketAddrV4> {
    let frame = forwarded(npdu);
    match receive(link, &frame, sender, to) {
        Received::Deliver(delivery) => {
            assert!(delivery.group, "a forwarded NPDU is a broadcast");
            Some(delivery.source)
        }
        _ => None,
    }
}

/// Each mode that hands a forward's NPDU up itself, with the sender and
/// destination it takes one from: a plain node from anyone, a foreign
/// device from its BBMD by unicast, or from anyone when told to.
fn receivers() -> [(&'static str, Link, SocketAddrV4, Addressed); 4] {
    let any_sender = with(foreign(), |foreign| {
        foreign.trust = ForwardedTrust::AnySender
    });
    [
        (
            "normal, by broadcast",
            link(Mode::Normal),
            PEER,
            TO_BROADCAST,
        ),
        ("normal, by unicast", link(Mode::Normal), PEER, UNICAST),
        (
            "foreign, from its BBMD",
            foreign_link(foreign()),
            BBMD,
            UNICAST,
        ),
        (
            "foreign, any sender",
            foreign_link(any_sender),
            PEER,
            UNICAST,
        ),
    ]
}

#[test]
fn an_answer_or_confirmed_request_inside_a_forward_is_never_shown_as_its_origins() {
    for (mode, link, sender, to) in receivers() {
        for header in [LOCAL, FROM_NETWORK_5] {
            for (kind, apdu) in DIRECTED {
                let frame = forwarded(&npdu(header, apdu));
                assert_eq!(
                    receive(&link, &frame, sender, to),
                    Received::Discarded(Discard::ForwardedNotBroadcast),
                    "{mode}: {kind} after {header:02X?}"
                );
            }
        }
    }
}

#[test]
fn broadcasts_inside_a_forward_are_still_taken_from_their_origin() {
    // I-Am-Router-To-Network for network 5: a network message.
    let router = [0x01, 0x80, 0x01, 0x00, 0x05];
    for (mode, link, sender, to) in receivers() {
        for header in [LOCAL, FROM_NETWORK_5, GLOBAL] {
            for (kind, apdu) in UNCONFIRMED {
                assert_eq!(
                    delivered_from(&link, &npdu(header, apdu), sender, to),
                    Some(REMOTE),
                    "{mode}: {kind} after {header:02X?}"
                );
            }
        }
        assert_eq!(
            delivered_from(&link, &router, sender, to),
            Some(REMOTE),
            "{mode}"
        );
    }
}

/// What a forward addressed to another network carries is the network
/// layer's to judge: a router may pass it on, a node drops it as not its
/// own, and a global broadcast of anything but an Unconfirmed-Request is
/// dropped there too (#1491). So is an NPDU that doesn't parse, and an
/// APDU of no defined type, which nothing decodes as an answer.
#[test]
fn a_forward_for_another_network_or_unreadable_goes_to_the_network_layer() {
    for (mode, link, sender, to) in receivers() {
        for (kind, apdu) in DIRECTED {
            for header in [TO_NETWORK_5, GLOBAL] {
                assert_eq!(
                    delivered_from(&link, &npdu(header, apdu), sender, to),
                    Some(REMOTE),
                    "{mode}: {kind} after {header:02X?}"
                );
            }
        }
        for unreadable in [&[0x01][..], &[0x01, 0x08, 0x00], &[0x01, 0x00, 0xAA, 0xBB]] {
            assert_eq!(
                delivered_from(&link, unreadable, sender, to),
                Some(REMOTE),
                "{mode}: {unreadable:02X?}"
            );
        }
    }
}

/// A BBMD hands its share of a peer's forward to the host, which forwards
/// it whatever it carries; whether the BBMD hands it up itself is the
/// host's to ask, with `forward_content_allowed`, the rule a node applies.
#[test]
fn a_bbmd_still_takes_every_peer_forward_to_pass_on() {
    let bbmd = link(Mode::Bbmd);
    for (kind, apdu) in DIRECTED {
        let npdu = npdu(LOCAL, apdu);
        let frame = forwarded(&npdu);
        assert!(matches!(
            receive(&bbmd, &frame, BBMD, UNICAST),
            Received::Bbmd(ForBbmd::Forwarded { origin: REMOTE, .. })
        ));
        assert!(!forward_content_allowed(&npdu), "{kind}");
    }
    for (kind, apdu) in UNCONFIRMED {
        assert!(forward_content_allowed(&npdu(LOCAL, apdu)), "{kind}");
    }
}
