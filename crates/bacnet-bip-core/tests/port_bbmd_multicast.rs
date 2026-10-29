//! A BBMD that serves a B/IP-M group (Annex J.8.3, #941): it is a member,
//! and the group stands in for its subnet's broadcast address, for what it
//! forwards there and for what it takes from there. Its own BDT row must be
//! two-hop.

mod node_support;

use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{BdtEntries, BdtEntry, Bvll, FdtEntry};
use bacnet_bip_core::bbmd::{BdtError, Nat, Reservation, TWO_HOP_MASK, Tables, Target};
use bacnet_bip_core::node::{Addressed, ConfigError, Discard, Link, Mode, Permissive};
use bacnet_bip_core::port::{BbmdPort, Ingress, PortError};
use bacnet_bip_core::{Instant, LocalSubnet};
use node_support::*;

/// The group the BBMD serves, at the network's port.
const GROUP: SocketAddrV4 = addr(239, 255, 0, 47, PORT);
/// A member of the group.
const MEMBER: SocketAddrV4 = addr(192, 168, 1, 40, PORT);
/// A peer BBMD on the BBMD's own IP subnet, at the network's port.
const NEIGHBOUR: SocketAddrV4 = addr(192, 168, 1, 30, PORT);
/// A one-hop peer, and its subnet's directed broadcast address.
const ONE_HOP: SocketAddrV4 = addr(192, 168, 2, 1, PORT);
const ONE_HOP_SUBNET: SocketAddrV4 = addr(192, 168, 2, 255, PORT);
/// Two foreign devices.
const FD1: SocketAddrV4 = addr(203, 0, 113, 7, PORT);
const FD2: SocketAddrV4 = addr(203, 0, 113, 8, PORT);

const MASK_24: Ipv4Addr = Ipv4Addr::new(255, 255, 255, 0);
const NPDU: &[u8] = &[0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];
const NO_ROW: FdtEntry = FdtEntry::new(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0), 0, 0);
const T0: Instant = Instant::from_secs(100);

type Small = Tables<4, 4>;

/// The BBMD's subnet: the group, its IP subnet's mask known.
fn group_subnet() -> LocalSubnet {
    let mut subnet = LocalSubnet::member(GROUP, Some(*BROADCAST.ip()));
    subnet.mask = Some(MASK_24);
    subnet
}

/// The BBMD's link, serving [`GROUP`].
fn group_link() -> Link {
    Link::new(OWN, group_subnet(), Mode::Bbmd).expect("a BBMD may serve a group")
}

/// Tables at [`OWN`] with the two-hop peers [`BBMD`] and [`NEIGHBOUR`], the
/// one-hop peer [`ONE_HOP`], and both foreign devices registered at `T0`.
fn tables() -> Small {
    let mut tables = Small::new(OWN);
    let peers = [
        BdtEntry::new(BBMD, TWO_HOP_MASK),
        BdtEntry::new(NEIGHBOUR, TWO_HOP_MASK),
        BdtEntry::new(ONE_HOP, MASK_24),
    ];
    tables.bdt.set(BdtEntries::from_entries(&peers)).unwrap();
    let limits = bacnet_bip_core::bbmd::RegistrationLimits::DEFAULT;
    for device in [FD1, FD2] {
        let outcome = tables
            .fdt
            .register(device, 60, T0, &limits, Reservation::Ordinary, || true);
        assert!(outcome.result.is_ok(), "{device}");
    }
    tables
}

/// Sent to the group.
const TO_GROUP: Addressed = Addressed {
    node: false,
    broadcast: true,
};

/// Run `body` against a port on `link` over [`tables`].
fn with_port(link: Link, body: impl FnOnce(&mut BbmdPort<'_, 4, 4>, &mut Socket)) {
    let mut tables = tables();
    let mut targets = [Target::UNUSED; Small::MAX_TARGETS];
    let mut rows = [NO_ROW; 4];
    let mut port = BbmdPort::new(link, &mut tables, &mut targets, &mut rows).expect("valid");
    let mut socket = Socket::default();
    body(&mut port, &mut socket);
}

/// Where each datagram went, with its message decoded as a Forwarded-NPDU's
/// origin.
fn forwarded_to(sent: &[(SocketAddrV4, Vec<u8>)], origin: SocketAddrV4) -> Vec<SocketAddrV4> {
    let message = wire(Bvll::ForwardedNpdu { origin, npdu: NPDU });
    sent.iter()
        .map(|(to, octets)| {
            assert_eq!(octets, &message, "to {to}");
            *to
        })
        .collect()
}

#[test]
fn a_bbmd_may_serve_a_group_and_a_foreign_device_may_not_join_one() {
    assert!(Link::new(OWN, LocalSubnet::new(GROUP), Mode::Bbmd).is_ok());
    assert_eq!(
        Link::new(OWN, LocalSubnet::new(GROUP), Mode::Foreign(foreign())),
        Err(ConfigError::MulticastMode)
    );
    assert!(group_subnet().is_group());
    assert!(!SUBNET.is_group());
}

/// J.8.3: an Original-Broadcast-NPDU that came through the group goes to
/// the other BDT entries and the foreign devices, not back to the group.
#[test]
fn a_broadcast_from_the_group_goes_to_the_bdt_and_the_fdt() {
    with_port(group_link(), |port, socket| {
        let payload = wire(Bvll::OriginalBroadcastNpdu(NPDU));
        let ingress = port.receive(
            &datagram(&payload, MEMBER, TO_GROUP),
            T0,
            &mut Permissive,
            socket,
        );
        let Ingress::Npdu(delivery) = ingress else {
            panic!("{ingress:?}");
        };
        assert_eq!((delivery.source, delivery.group), (MEMBER, true));
        assert_eq!(
            forwarded_to(&socket.take(), MEMBER),
            [BBMD, NEIGHBOUR, ONE_HOP_SUBNET, FD1, FD2]
        );

        // A broadcast to the IP subnet isn't this network's: a plain device
        // there belongs to another one (J.8.1).
        let to_subnet = Addressed::classify(
            *BROADCAST.ip(),
            None,
            &bacnet_bip_core::node::LocalAddresses::new(*OWN.ip(), *GROUP.ip()),
        );
        let ingress = port.receive(
            &datagram(&payload, MEMBER, to_subnet),
            T0,
            &mut Permissive,
            socket,
        );
        assert_eq!(ingress, Ingress::Discarded(Discard::DestinationMismatch));
        assert!(socket.take().is_empty());
    });
}

/// J.8.3: a peer's Forwarded-NPDU goes to the group and the foreign
/// devices. A plain peer on the BBMD's own IP subnet is outside the
/// standard (J.8.1); its forwards still reach the group, the one direction
/// that topology is tolerated in.
#[test]
fn a_peers_forward_goes_to_the_group_and_the_fdt() {
    let payload = wire(Bvll::ForwardedNpdu {
        origin: REMOTE,
        npdu: NPDU,
    });
    for peer in [BBMD, NEIGHBOUR] {
        with_port(group_link(), |port, socket| {
            let ingress = port.receive(
                &datagram(&payload, peer, UNICAST),
                T0,
                &mut Permissive,
                socket,
            );
            let Ingress::Npdu(delivery) = ingress else {
                panic!("{peer}: {ingress:?}");
            };
            assert_eq!(delivery.source, REMOTE);
            assert_eq!(
                forwarded_to(&socket.take(), REMOTE),
                [GROUP, FD1, FD2],
                "{peer}"
            );
        });
    }
    // A plain BBMD with the same tables leaves the neighbour's forward to
    // the subnet, which heard it first hand (J.4.5).
    let mut plain_subnet = SUBNET;
    plain_subnet.mask = Some(MASK_24);
    let plain = Link::new(OWN, plain_subnet, Mode::Bbmd).unwrap();
    with_port(plain, |port, socket| {
        port.receive(
            &datagram(&payload, NEIGHBOUR, UNICAST),
            T0,
            &mut Permissive,
            socket,
        );
        assert_eq!(forwarded_to(&socket.take(), REMOTE), [FD1, FD2]);
    });
}

/// J.8.3: a foreign device's Distribute-Broadcast-To-Network goes to the
/// group, the BDT and every other foreign device, never back to it.
#[test]
fn a_foreign_devices_broadcast_goes_to_the_group_the_bdt_and_the_other_devices() {
    with_port(group_link(), |port, socket| {
        let payload = wire(Bvll::DistributeBroadcastToNetwork(NPDU));
        let ingress = port.receive(
            &datagram(&payload, FD1, UNICAST),
            T0,
            &mut Permissive,
            socket,
        );
        let Ingress::Npdu(delivery) = ingress else {
            panic!("{ingress:?}");
        };
        assert_eq!(delivery.source, FD1);
        assert_eq!(
            forwarded_to(&socket.take(), FD1),
            [GROUP, BBMD, NEIGHBOUR, ONE_HOP_SUBNET, FD2]
        );
    });
}

/// The BBMD's own broadcast goes to the group, as a member's does, and on
/// to the BDT and the foreign devices (J.4.5, #937).
#[test]
fn the_bbmds_own_broadcast_goes_to_the_group_then_the_bdt_and_fdt() {
    with_port(group_link(), |port, socket| {
        port.broadcast(NPDU, T0, socket).unwrap();
        let sent = socket.take();
        assert_eq!(sent[0], (GROUP, wire(Bvll::OriginalBroadcastNpdu(NPDU))));
        assert_eq!(
            forwarded_to(&sent[1..], OWN),
            [BBMD, NEIGHBOUR, ONE_HOP_SUBNET, FD1, FD2]
        );
    });
}

/// The BBMD's own copies come back through the group, and are dropped.
#[test]
fn the_bbmds_echoes_from_the_group_are_dropped() {
    with_port(group_link(), |port, socket| {
        let relayed = wire(Bvll::ForwardedNpdu {
            origin: REMOTE,
            npdu: NPDU,
        });
        let own = wire(Bvll::OriginalBroadcastNpdu(NPDU));
        for payload in [&relayed, &own] {
            let ingress = port.receive(
                &datagram(payload, OWN, TO_GROUP),
                T0,
                &mut Permissive,
                socket,
            );
            assert_eq!(ingress, Ingress::Discarded(Discard::OwnEcho));
        }
        assert!(socket.take().is_empty());
    });
}

/// J.8.3: the BBMD's own row must have an all-ones mask, which makes the
/// peers send it the network's broadcasts itself.
#[test]
fn a_bbmd_that_serves_a_group_needs_a_two_hop_own_row() {
    let one_hop_own = BdtEntry::new(OWN, MASK_24);
    let mut tables = Small::new(OWN);
    tables
        .bdt
        .set(BdtEntries::from_entries(&[one_hop_own]))
        .unwrap();
    let mut targets = [Target::UNUSED; Small::MAX_TARGETS];
    let mut rows = [NO_ROW; 4];
    let refused = BbmdPort::new(group_link(), &mut tables, &mut targets, &mut rows).err();
    assert_eq!(
        refused,
        Some(PortError::Bdt(BdtError::OneHopForGroup {
            entry: one_hop_own
        }))
    );
    assert!(!tables.bdt.serves_group(), "unchanged");

    // A table with no own row gets a two-hop one.
    tables.bdt.set(BdtEntries::from_entries(&[])).unwrap();
    tables.bdt.set_serves_group(true).unwrap();
    assert_eq!(tables.bdt.row(OWN), Some(BdtEntry::new(OWN, TWO_HOP_MASK)));
    // From then on a one-hop own row is refused, and nothing changes.
    let before = tables.bdt.rows().to_vec();
    assert_eq!(
        tables.bdt.set(BdtEntries::from_entries(&[one_hop_own])),
        Err(BdtError::OneHopForGroup { entry: one_hop_own })
    );
    assert_eq!(tables.bdt.rows(), before);
    // Moving the own address onto a one-hop row is refused too.
    let moved = BdtEntry::new(NEIGHBOUR, MASK_24);
    tables.bdt.set(BdtEntries::from_entries(&[moved])).unwrap();
    assert_eq!(
        tables.bdt.set_own_address(NEIGHBOUR),
        Err(BdtError::OneHopForGroup { entry: moved })
    );
    assert_eq!(tables.bdt.own_address(), OWN);
    // One-hop rows for the peers are fine.
    tables
        .bdt
        .set(BdtEntries::from_entries(&[BdtEntry::new(ONE_HOP, MASK_24)]))
        .unwrap();
    // A plain BBMD takes the one-hop own row again.
    tables.bdt.set_serves_group(false).unwrap();
    tables
        .bdt
        .set(BdtEntries::from_entries(&[one_hop_own]))
        .unwrap();
    // And a port on a plain link clears the rule.
    tables.bdt.set_serves_group(true).unwrap_err();
    let plain = Link::new(OWN, SUBNET, Mode::Bbmd).unwrap();
    assert!(BbmdPort::new(plain, &mut tables, &mut targets, &mut rows).is_ok());
}

/// A BBMD behind a NAT router may serve a group: both rules want a
/// two-hop own row, and the NAT rules for the origin are unchanged (Annex
/// J.7.5). The group keeps its members' broadcasts on the private side.
#[test]
fn a_group_and_nat_traversal_compose() {
    let global = addr(203, 0, 113, 1, PORT);
    let mut tables = Small::new(OWN);
    tables.bdt.set_serves_group(true).unwrap();
    let rows_with_global = [
        BdtEntry::new(global, TWO_HOP_MASK),
        BdtEntry::new(BBMD, TWO_HOP_MASK),
    ];
    tables
        .set_nat_and_bdt(
            Some(Nat::new(global)),
            OWN,
            BdtEntries::from_entries(&rows_with_global),
        )
        .unwrap();
    assert!(tables.bdt.serves_group(), "kept across the NAT change");
    let mut targets = [Target::UNUSED; Small::MAX_TARGETS];
    let mut rows = [NO_ROW; 4];
    let mut port = BbmdPort::new(group_link(), &mut tables, &mut targets, &mut rows).unwrap();
    let mut socket = Socket::default();

    // A member's broadcast reaches the peer under the global address.
    let payload = wire(Bvll::OriginalBroadcastNpdu(NPDU));
    port.receive(
        &datagram(&payload, MEMBER, TO_GROUP),
        T0,
        &mut Permissive,
        &mut socket,
    );
    assert_eq!(forwarded_to(&socket.take(), global), [BBMD]);
    // A peer's forward goes to the group with its origin as it came.
    let payload = wire(Bvll::ForwardedNpdu {
        origin: REMOTE,
        npdu: NPDU,
    });
    port.receive(
        &datagram(&payload, BBMD, UNICAST),
        T0,
        &mut Permissive,
        &mut socket,
    );
    assert_eq!(forwarded_to(&socket.take(), REMOTE), [GROUP]);
}

/// A peer's Forwarded-NPDU that came through the group (J.4.5): the group
/// heard it already, so it goes to the foreign devices only, never back to
/// the group, which would loop it.
#[test]
fn a_forward_from_the_group_never_goes_back_to_it() {
    with_port(group_link(), |port, socket| {
        let payload = wire(Bvll::ForwardedNpdu {
            origin: REMOTE,
            npdu: NPDU,
        });
        let ingress = port.receive(
            &datagram(&payload, BBMD, TO_GROUP),
            T0,
            &mut Permissive,
            socket,
        );
        assert!(matches!(ingress, Ingress::Npdu(_)), "{ingress:?}");
        assert_eq!(forwarded_to(&socket.take(), REMOTE), [FD1, FD2]);
    });
}

/// The port checks, in debug builds, that the BDT's group rule still
/// matches its link: a caller that turned it off through `tables_mut`
/// broke the pairing `new` made.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "group rule")]
fn a_group_rule_that_drifts_from_the_link_is_caught() {
    with_port(group_link(), |port, socket| {
        port.tables_mut().bdt.set_serves_group(false).unwrap();
        let payload = wire(Bvll::OriginalBroadcastNpdu(NPDU));
        port.receive(
            &datagram(&payload, MEMBER, TO_GROUP),
            T0,
            &mut Permissive,
            socket,
        );
    });
}
