//! The receive rules of `Link::receive`, in their order, and the dispatch of
//! each function by mode (Annex J.1.2, J.2, J.4).

mod node_support;

use core::net::Ipv4Addr;

use bacnet_bip_codec::{BdtEntries, BdtEntry, BvlcFunction, BvlcResultCode, Bvll, FdtEntries};
use bacnet_bip_core::node::{
    Addressed, Delivery, Discard, ForBbmd, ForwardedTrust, Inbound, Link, LocalAddresses, Mode,
    Node, Path, Permissive, Received, Response,
};
use bacnet_bip_core::{Arrival, Instant, LocalSubnet};
use node_support::*;

const NPDU: &[u8] = &[0x01, 0x00, 0x10, 0x08];

fn normal() -> Link {
    link(Mode::Normal)
}

/// A delivery of [`NPDU`] a test expects. [`Delivery`] is
/// `#[non_exhaustive]`, so a test outside the crate can't build one to
/// compare with: a `Delivery` or a `Received` compares with this instead.
#[derive(Debug)]
struct Delivered {
    source: core::net::SocketAddrV4,
    group: bool,
}

impl PartialEq<Delivered> for Delivery<'_> {
    fn eq(&self, expected: &Delivered) -> bool {
        self.npdu == NPDU && self.source == expected.source && self.group == expected.group
    }
}

impl PartialEq<Delivered> for Received<'_> {
    fn eq(&self, expected: &Delivered) -> bool {
        matches!(self, Received::Deliver(delivery) if delivery == expected)
    }
}

fn delivered(source: core::net::SocketAddrV4, group: bool) -> Delivered {
    Delivered { source, group }
}

#[test]
fn a_peer_on_another_port_is_delivered_and_answered_there() {
    let link = normal();
    let peer = addr(192, 168, 1, 20, 50_000);
    let unicast = wire(Bvll::OriginalUnicastNpdu(NPDU));
    assert_eq!(
        receive(&link, &unicast, peer, UNICAST),
        delivered(peer, false)
    );
    let read = wire(Bvll::ReadBroadcastDistributionTable);
    assert_eq!(
        receive(&link, &read, peer, UNICAST),
        Received::Nak {
            to: peer,
            code: BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK,
            malformed: false
        }
    );
    // The node's own IP at another port is a peer, not its echo.
    let neighbour = addr(192, 168, 1, 10, 47_809);
    assert_eq!(
        receive(&link, &unicast, neighbour, UNICAST),
        delivered(neighbour, false)
    );
}

#[test]
fn an_ephemeral_local_port_is_the_nodes_own() {
    let own = addr(192, 168, 1, 10, 54_321);
    let link = Link::new(own, SUBNET, Mode::Normal).unwrap();
    let unicast = wire(Bvll::OriginalUnicastNpdu(NPDU));
    assert_eq!(
        receive(&link, &unicast, own, UNICAST),
        Received::Discarded(Discard::OwnEcho)
    );
    assert_eq!(
        receive(&link, &unicast, OWN, UNICAST),
        delivered(OWN, false)
    );
    let read = wire(Bvll::ReadForeignDeviceTable);
    assert!(matches!(
        receive(&link, &read, OWN, UNICAST),
        Received::Nak { to, .. } if to == OWN
    ));
}

#[test]
fn header_failures_are_discarded_by_reason() {
    let link = normal();
    let cases: [(&[u8], Discard); 4] = [
        (&[0x81, 0x0A, 0x00], Discard::TooShort),
        (&[0x82, 0x0A, 0x00, 0x05, 0x01], Discard::NotBip),
        // The Length field counts the whole message (Annex J.2).
        (&[0x81, 0x0A, 0x00, 0x06, 0x01], Discard::LengthMismatch),
        (&[0x81, 0x0A, 0x00, 0x04, 0x01], Discard::LengthMismatch),
    ];
    for (payload, reason) in cases {
        assert_eq!(
            receive(&link, payload, PEER, UNICAST),
            Received::Discarded(reason),
            "{payload:02X?}"
        );
    }
}

#[test]
fn forwarded_npdu_is_delivered_from_its_origin() {
    let link = normal();
    let forwarded = wire(Bvll::ForwardedNpdu {
        origin: REMOTE,
        npdu: NPDU,
    });
    for to in [UNICAST, TO_BROADCAST] {
        assert_eq!(
            receive(&link, &forwarded, BBMD, to),
            delivered(REMOTE, true)
        );
    }
}

#[test]
fn original_functions_must_fit_their_destination() {
    let link = normal();
    let unicast = wire(Bvll::OriginalUnicastNpdu(NPDU));
    let broadcast = wire(Bvll::OriginalBroadcastNpdu(NPDU));
    let both = Addressed {
        node: true,
        broadcast: true,
    };
    let mismatch = Received::Discarded(Discard::DestinationMismatch);
    assert_eq!(receive(&link, &unicast, PEER, TO_BROADCAST), mismatch);
    assert_eq!(receive(&link, &broadcast, PEER, UNICAST), mismatch);
    assert_eq!(
        receive(&link, &unicast, PEER, Addressed::default()),
        mismatch
    );
    assert_eq!(
        receive(&link, &unicast, PEER, UNICAST),
        delivered(PEER, false)
    );
    assert_eq!(
        receive(&link, &broadcast, PEER, TO_BROADCAST),
        delivered(PEER, true)
    );
    // Where the broadcast address is the node's own, the function decides.
    assert_eq!(receive(&link, &unicast, PEER, both), delivered(PEER, false));
    assert_eq!(
        receive(&link, &broadcast, PEER, both),
        delivered(PEER, true)
    );
    // Management traffic is point to point.
    assert_eq!(
        receive(&link, &result(SUCCESS), PEER, TO_BROADCAST),
        mismatch
    );
}

#[test]
fn unknown_functions_are_dropped_unanswered() {
    for mode in [Mode::Normal, Mode::Foreign(foreign()), Mode::Bbmd] {
        let link = link(mode);
        for function in [0x0C, 0x0D, 0x80, 0xFF] {
            let frame = raw(function, &[0xAA, 0xBB]);
            assert_eq!(
                receive(&link, &frame, PEER, UNICAST),
                Received::Discarded(Discard::UnknownFunction),
                "{function:#04X}"
            );
        }
    }
}

#[test]
fn group_sources_and_origins_are_dropped() {
    let link = normal();
    let unicast = wire(Bvll::OriginalUnicastNpdu(NPDU));
    for source in [
        addr(255, 255, 255, 255, PORT),
        addr(224, 0, 0, 1, PORT),
        addr(239, 255, 0, 1, 50_000),
        // The broadcast IP, at any port.
        addr(192, 168, 1, 255, PORT),
        addr(192, 168, 1, 255, 50_000),
    ] {
        assert_eq!(
            receive(&link, &unicast, source, UNICAST),
            Received::Discarded(Discard::GroupSource),
            "{source}"
        );
    }
    for origin in [
        BROADCAST,
        addr(192, 168, 1, 255, 1),
        addr(224, 0, 0, 9, PORT),
    ] {
        let forwarded = wire(Bvll::ForwardedNpdu { origin, npdu: NPDU });
        assert_eq!(
            receive(&link, &forwarded, BBMD, UNICAST),
            Received::Discarded(Discard::GroupOrigin),
            "{origin}"
        );
    }
    // A loopback broadcast IP, as loopback tests set up, is a group only at
    // the network's port.
    let loopback = LocalSubnet::new(addr(127, 0, 0, 1, PORT));
    let link = Link::new(addr(127, 0, 0, 1, 47_809), loopback, Mode::Normal).unwrap();
    let peer = addr(127, 0, 0, 1, 47_810);
    assert_eq!(
        receive(&link, &unicast, peer, UNICAST),
        delivered(peer, false)
    );
    let forwarded = wire(Bvll::ForwardedNpdu {
        origin: addr(127, 0, 0, 1, PORT),
        npdu: NPDU,
    });
    assert_eq!(
        receive(&link, &forwarded, peer, UNICAST),
        Received::Discarded(Discard::GroupOrigin)
    );
}

#[test]
fn own_echoes_are_dropped() {
    let echo = Received::Discarded(Discard::OwnEcho);
    let own_origin = wire(Bvll::ForwardedNpdu {
        origin: OWN,
        npdu: NPDU,
    });
    for mode in [Mode::Normal, Mode::Foreign(foreign()), Mode::Bbmd] {
        let link = link(mode);
        let unicast = wire(Bvll::OriginalUnicastNpdu(NPDU));
        assert_eq!(receive(&link, &unicast, OWN, UNICAST), echo);
        let broadcast = wire(Bvll::OriginalBroadcastNpdu(NPDU));
        assert_eq!(receive(&link, &broadcast, OWN, TO_BROADCAST), echo);
        assert_eq!(receive(&link, &own_origin, BBMD, UNICAST), echo);
        let distribute = wire(Bvll::DistributeBroadcastToNetwork(NPDU));
        assert_eq!(receive(&link, &distribute, OWN, UNICAST), echo);
    }
    // A BBMD's own rebroadcast keeps its remote origin.
    let rebroadcast = wire(Bvll::ForwardedNpdu {
        origin: REMOTE,
        npdu: NPDU,
    });
    assert_eq!(
        receive(&link(Mode::Bbmd), &rebroadcast, OWN, TO_BROADCAST),
        echo
    );
    // A node that is no BBMD delivers that one: only its origin matters.
    assert_eq!(
        receive(&normal(), &rebroadcast, OWN, TO_BROADCAST),
        delivered(REMOTE, true)
    );
}

fn on_listener(link: &Link, payload: &[u8], to: Addressed, other_interface: bool) -> Discard {
    let datagram = with(datagram(payload, PEER, to), |datagram| {
        datagram.path = Path::Listener { other_interface };
    });
    match receive_datagram(link, &datagram) {
        Received::Discarded(reason) => reason,
        Received::Deliver(_) => Discard::Truncated,
        other => panic!("{other:?}"),
    }
}

#[test]
fn listeners_keep_only_this_links_broadcasts() {
    let link = normal();
    let broadcast = wire(Bvll::OriginalBroadcastNpdu(NPDU));
    let unicast = wire(Bvll::OriginalUnicastNpdu(NPDU));
    let mismatch = |other_interface| Discard::ListenerMismatch { other_interface };
    assert_eq!(
        on_listener(&link, &unicast, UNICAST, false),
        mismatch(false)
    );
    // Whatever it holds: a listener reads nothing it doesn't keep.
    assert_eq!(on_listener(&link, &[0x00], UNICAST, false), mismatch(false));
    assert_eq!(
        on_listener(&link, &broadcast, TO_BROADCAST, true),
        mismatch(true)
    );
    let both = Addressed {
        node: true,
        broadcast: true,
    };
    assert_eq!(on_listener(&link, &unicast, both, false), mismatch(false));
    let kept = with(datagram(&broadcast, PEER, TO_BROADCAST), |datagram| {
        datagram.path = Path::Listener {
            other_interface: false,
        };
    });
    assert_eq!(receive_datagram(&link, &kept), delivered(PEER, true));
}

#[test]
fn wildcard_binds_take_unicast_only_at_own_addresses() {
    let own_ips = [Ipv4Addr::new(10, 0, 0, 5), Ipv4Addr::new(192, 168, 1, 10)];
    let bound = LocalAddresses::new(
        Ipv4Addr::new(192, 168, 1, 10),
        Ipv4Addr::new(192, 168, 1, 255),
    );
    let wildcard = with(bound, |local| local.wildcard = Some(&own_ips));
    let classify = |ip: [u8; 4], os, local: &LocalAddresses<'_>| {
        let addressed = Addressed::classify(Ipv4Addr::from(ip), os, local);
        (addressed.node, addressed.broadcast)
    };
    assert_eq!(classify([10, 0, 0, 5], None, &wildcard), (true, false));
    assert_eq!(classify([10, 0, 0, 6], None, &wildcard), (false, false));
    assert_eq!(classify([192, 168, 1, 255], None, &wildcard), (false, true));
    assert_eq!(
        classify([255, 255, 255, 255], None, &wildcard),
        (false, true)
    );
    assert_eq!(classify([224, 0, 0, 1], None, &wildcard), (false, false));
    assert_eq!(
        classify([10, 0, 0, 5], Some(true), &wildcard),
        (false, false)
    );
    assert_eq!(
        classify([10, 0, 0, 5], Some(false), &wildcard),
        (true, false)
    );
    assert_eq!(
        classify([192, 168, 1, 255], Some(false), &wildcard),
        (false, false)
    );
    assert_eq!(classify([192, 168, 1, 10], None, &bound), (true, false));
    assert_eq!(classify([10, 0, 0, 5], None, &bound), (false, false));
    // A wildcard bind never counts the broadcast address as its own, even
    // when the host lists it.
    let odd = [Ipv4Addr::new(192, 168, 1, 255)];
    let listed = with(wildcard, |local| local.wildcard = Some(&odd));
    assert_eq!(classify([192, 168, 1, 255], None, &listed), (false, true));
    // A bound socket whose broadcast address is its own, as in loopback
    // tests, has both.
    let loopback = LocalAddresses::new(Ipv4Addr::LOCALHOST, Ipv4Addr::LOCALHOST);
    assert_eq!(classify([127, 0, 0, 1], None, &loopback), (true, true));
}

#[test]
fn the_policy_sees_covered_requests_before_their_payload() {
    let link = normal();
    let mut refusing = Recording::new(false, true);
    // Each with one stray octet: refused before the payload is read, so
    // rate limited, not malformed.
    for function in [0x02, 0x05, 0x06, 0x08] {
        let frame = raw(function, &[0x01]);
        let datagram = datagram(&frame, PEER, UNICAST);
        assert_eq!(
            link.receive(&datagram, Instant::ZERO, &mut refusing),
            Received::Discarded(Discard::RateLimited)
        );
    }
    // Write-BDT, Distribute-Broadcast-To-Network and the data functions are
    // never asked about.
    let write = wire(Bvll::WriteBroadcastDistributionTable(
        BdtEntries::from_entries(&[]),
    ));
    let distribute = wire(Bvll::DistributeBroadcastToNetwork(NPDU));
    let unicast = wire(Bvll::OriginalUnicastNpdu(NPDU));
    for frame in [&write, &distribute, &unicast] {
        let datagram = datagram(frame, PEER, UNICAST);
        assert!(!matches!(
            link.receive(&datagram, Instant::ZERO, &mut refusing),
            Received::Discarded(_)
        ));
    }
    assert_eq!(
        refusing.asked,
        [
            BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE,
            BvlcFunction::REGISTER_FOREIGN_DEVICE,
            BvlcFunction::READ_FOREIGN_DEVICE_TABLE,
            BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY,
        ]
    );
    // A table ACK the policy refuses is a response it didn't accept.
    let rows = [BdtEntry::new(PEER, Ipv4Addr::BROADCAST)];
    let ack = wire(Bvll::ReadBroadcastDistributionTableAck(
        BdtEntries::from_entries(&rows),
    ));
    let mut picky = Recording::new(true, false);
    let datagram = datagram(&ack, PEER, UNICAST);
    assert!(matches!(
        link.receive(&datagram, Instant::ZERO, &mut picky),
        Received::Response(Response {
            accepted: false,
            message: Ok(_),
            ..
        })
    ));
    let fdt_ack = wire(Bvll::ReadForeignDeviceTableAck(FdtEntries::from_entries(
        &[],
    )));
    assert!(matches!(
        receive(&link, &fdt_ack, PEER, UNICAST),
        Received::Response(Response { accepted: true, .. })
    ));
}

#[test]
fn responses_go_on_to_be_matched_malformed_or_not() {
    let link = normal();
    let success = result(SUCCESS);
    let received = receive(&link, &success, BBMD, UNICAST);
    let Received::Response(response) = received else {
        panic!("{received:?}");
    };
    assert_eq!(
        (
            response.peer,
            response.function,
            response.message,
            response.accepted
        ),
        (
            BBMD,
            BvlcFunction::BVLC_RESULT,
            Ok(Bvll::BvlcResult(SUCCESS)),
            true
        )
    );
    for frame in [
        raw(0x00, &[0x00]),
        raw(0x03, &[0x00; 7]),
        raw(0x07, &[0x00; 11]),
    ] {
        assert!(matches!(
            receive(&link, &frame, BBMD, UNICAST),
            Received::Response(Response {
                message: Err(_),
                accepted: false,
                ..
            })
        ));
    }
    // A malformed data function is dropped.
    assert_eq!(
        receive(&link, &raw(0x04, &[1, 2, 3]), BBMD, UNICAST),
        Received::Discarded(Discard::MalformedPayload)
    );
}

#[test]
fn truncation_is_checked_first() {
    let link = normal();
    let source = addr(255, 255, 255, 255, PORT);
    let truncated = with(datagram(&[0x00], source, Addressed::default()), |d| {
        d.truncated = true;
        d.path = Path::Listener {
            other_interface: true,
        };
    });
    assert_eq!(
        receive_datagram(&link, &truncated),
        Received::Discarded(Discard::Truncated)
    );
    // Then the group source, before the listener.
    let grouped = with(truncated, |d| d.truncated = false);
    assert_eq!(
        receive_datagram(&link, &grouped),
        Received::Discarded(Discard::GroupSource)
    );
}

/// A datagram is dropped, never delivered on a guess, when its destination
/// doesn't fit its function, when the host couldn't say what its
/// destination was (#1678), when it was cut short, and when its Length
/// field is wrong.
///
/// Ported from ACE IoT's bacnet-router-adapters
/// (ACE-IoT-Solutions/rusty-bacnet#5), thanks @acedrew, where it is
/// `ingress_fails_closed_for_destination_mismatch_missing_metadata_and_truncation`.
#[test]
fn ingress_fails_closed_for_destination_mismatch_missing_metadata_and_truncation() {
    let unicast = wire(Bvll::OriginalUnicastNpdu(NPDU));
    let dropped = |reason| Received::Discarded(reason);
    assert_eq!(
        receive(&normal(), &unicast, PEER, TO_BROADCAST),
        dropped(Discard::DestinationMismatch)
    );

    // No destination: dropped in every mode, whatever the function, the
    // socket or the sender, and before the frame is read.
    let broadcast = wire(Bvll::OriginalBroadcastNpdu(NPDU));
    let forwarded = wire(Bvll::ForwardedNpdu {
        origin: REMOTE,
        npdu: NPDU,
    });
    let register = wire(Bvll::RegisterForeignDevice { ttl: 60 });
    let modes = [
        Mode::Normal,
        Mode::Bbmd,
        Mode::Foreign(with(foreign(), |f| f.trust = ForwardedTrust::AnySender)),
    ];
    let paths = [
        Path::Primary,
        Path::Listener {
            other_interface: false,
        },
    ];
    for mode in modes {
        let link = link(mode);
        for payload in [&unicast, &broadcast, &forwarded, &register, &vec![0x81]] {
            for (path, source) in paths.into_iter().zip([PEER, BBMD]) {
                let unknown = with(datagram(payload, source, UNICAST), |d| {
                    d.to = None;
                    d.path = path;
                });
                assert_eq!(
                    receive_datagram(&link, &unknown),
                    dropped(Discard::UnknownDestination),
                    "{mode:?} {path:?} {payload:02x?}"
                );
            }
        }
    }
    // The node engine delivers nothing from one, and has nothing to do.
    let mut node: Node = Node::new(normal(), Instant::ZERO).unwrap();
    let unknown = with(datagram(&unicast, PEER, UNICAST), |d| d.to = None);
    assert_eq!(
        node.receive(&unknown, Instant::ZERO, &mut Permissive),
        Inbound::Discarded(Discard::UnknownDestination)
    );
    assert!(drain(&mut node).is_empty());

    // A cut-short datagram is dropped as that, even with no destination;
    // a group source comes next.
    let truncated = with(unknown, |d| d.truncated = true);
    assert_eq!(
        receive_datagram(&normal(), &truncated),
        dropped(Discard::Truncated)
    );
    let grouped = with(unknown, |d| d.source = BROADCAST);
    assert_eq!(
        receive_datagram(&normal(), &grouped),
        dropped(Discard::GroupSource)
    );

    // A Length field one past the datagram's length.
    let mut long = unicast.clone();
    long[3] += 1;
    assert_eq!(
        receive(&normal(), &long, PEER, UNICAST),
        dropped(Discard::LengthMismatch)
    );
}

#[test]
fn foreign_trust_takes_forwards_only_from_the_bbmd_by_unicast() {
    let forwarded = wire(Bvll::ForwardedNpdu {
        origin: REMOTE,
        npdu: NPDU,
    });
    let untrusted = Received::Discarded(Discard::ForwardedUntrusted);
    let strict = link(Mode::Foreign(foreign()));
    assert_eq!(
        receive(&strict, &forwarded, BBMD, UNICAST),
        delivered(REMOTE, true)
    );
    assert_eq!(receive(&strict, &forwarded, BBMD, TO_BROADCAST), untrusted);
    assert_eq!(receive(&strict, &forwarded, PEER, UNICAST), untrusted);
    // The BBMD's IP at another port is another sender.
    let other_port = addr(10, 0, 0, 1, 47_809);
    assert_eq!(receive(&strict, &forwarded, other_port, UNICAST), untrusted);
    let mut open = foreign();
    open.trust = ForwardedTrust::AnySender;
    let open = link(Mode::Foreign(open));
    assert_eq!(
        receive(&open, &forwarded, PEER, TO_BROADCAST),
        delivered(REMOTE, true)
    );
    assert_eq!(
        receive(&normal(), &forwarded, PEER, UNICAST),
        delivered(REMOTE, true)
    );
}

#[test]
fn bbmd_mode_hands_over_its_share() {
    let link = link(Mode::Bbmd);
    let broadcast = wire(Bvll::OriginalBroadcastNpdu(NPDU));
    let received = receive(&link, &broadcast, PEER, TO_BROADCAST);
    let Received::Bbmd(ForBbmd::OriginalBroadcast(delivery)) = received else {
        panic!("{received:?}");
    };
    assert_eq!(delivery, delivered(PEER, true));
    let forwarded = wire(Bvll::ForwardedNpdu {
        origin: REMOTE,
        npdu: NPDU,
    });
    for (to, arrival) in [
        (UNICAST, Arrival::Unicast),
        (TO_BROADCAST, Arrival::Broadcast),
    ] {
        assert_eq!(
            receive(&link, &forwarded, BBMD, to),
            Received::Bbmd(ForBbmd::Forwarded {
                sender: BBMD,
                arrival,
                origin: REMOTE,
                npdu: NPDU
            })
        );
    }
    let shares = [
        (
            wire(Bvll::DistributeBroadcastToNetwork(NPDU)),
            ForBbmd::Distribute {
                sender: PEER,
                npdu: NPDU,
            },
        ),
        (
            wire(Bvll::RegisterForeignDevice { ttl: 90 }),
            ForBbmd::Register {
                sender: PEER,
                ttl: 90,
            },
        ),
        (
            wire(Bvll::ReadBroadcastDistributionTable),
            ForBbmd::ReadBdt { sender: PEER },
        ),
        (
            wire(Bvll::ReadForeignDeviceTable),
            ForBbmd::ReadFdt { sender: PEER },
        ),
        (
            wire(Bvll::DeleteForeignDeviceTableEntry(REMOTE)),
            ForBbmd::DeleteFdtEntry {
                sender: PEER,
                entry: REMOTE,
            },
        ),
    ];
    for (payload, share) in shares {
        assert_eq!(
            receive(&link, &payload, PEER, UNICAST),
            Received::Bbmd(share)
        );
    }
    // Unicast data stays the node's.
    let unicast = wire(Bvll::OriginalUnicastNpdu(NPDU));
    assert_eq!(
        receive(&link, &unicast, PEER, UNICAST),
        delivered(PEER, false)
    );
}
