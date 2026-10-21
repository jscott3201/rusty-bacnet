//! The port driver on a BBMD behind a NAT router (Annex J.7.5): the
//! Forwarded-NPDUs it puts on the wire carry the global address as their
//! origin when it builds them, and the origin they came with when it passes
//! a peer's on.

mod node_support;

use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{BdtEntries, BdtEntry, Bvll, FdtEntry};
use bacnet_bip_core::bbmd::{Nat, Reservation, Tables, Target, TWO_HOP_MASK};
use bacnet_bip_core::node::{Mode, Permissive};
use bacnet_bip_core::port::{BbmdPort, Ingress};
use node_support::*;

/// The NAT router's public address, forwarded to [`OWN`].
const GLOBAL: SocketAddrV4 = addr(203, 0, 113, 1, PORT);
/// The other site's router.
const OTHER_SITE: SocketAddrV4 = addr(198, 51, 100, 2, PORT);
/// A device at the other site, behind its router.
const BEHIND_OTHER: SocketAddrV4 = addr(192, 168, 7, 9, PORT);
/// A foreign device registered from the public side.
const ROAMING: SocketAddrV4 = addr(198, 51, 100, 77, PORT);
/// A foreign device registered from this BBMD's private side.
const INSIDE: SocketAddrV4 = addr(192, 168, 1, 50, PORT);

const NPDU: &[u8] = &[0x01, 0x00, 0x10, 0x08];
const NO_ROW: FdtEntry = FdtEntry::new(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0), 0, 0);

type Small = Tables<2, 2>;

fn tables() -> Small {
    let mut tables = Small::new(OWN);
    tables.set_nat(Some(Nat::new(GLOBAL)), OWN).unwrap();
    let bdt = [
        BdtEntry::new(GLOBAL, TWO_HOP_MASK),
        BdtEntry::new(OTHER_SITE, TWO_HOP_MASK),
    ];
    tables.bdt.set(BdtEntries::from_entries(&bdt)).unwrap();
    for device in [ROAMING, INSIDE] {
        let limits = bacnet_bip_core::bbmd::RegistrationLimits::DEFAULT;
        let outcome =
            tables
                .fdt
                .register(device, 60, at(0), &limits, Reservation::Ordinary, || true);
        assert!(outcome.result.is_ok());
    }
    tables
}

fn forwarded(origin: SocketAddrV4) -> Vec<u8> {
    wire(Bvll::ForwardedNpdu { origin, npdu: NPDU })
}

#[test]
fn what_the_port_builds_carries_the_global_address() {
    let mut tables = tables();
    let mut targets = [Target::UNUSED; Small::MAX_TARGETS];
    let mut rows = [NO_ROW; 2];
    let mut port = BbmdPort::new(link(Mode::Bbmd), &mut tables, &mut targets, &mut rows).unwrap();
    let mut socket = Socket::default();

    // Its own broadcast: the subnet, then the other site and both foreign
    // devices, all from the global address.
    port.broadcast(NPDU, at(1_000), &mut socket).unwrap();
    assert_eq!(
        socket.take(),
        [
            (BROADCAST, wire(Bvll::OriginalBroadcastNpdu(NPDU))),
            (OTHER_SITE, forwarded(GLOBAL)),
            (ROAMING, forwarded(GLOBAL)),
            (INSIDE, forwarded(GLOBAL)),
        ]
    );

    // A local device's broadcast, handed up from the device itself.
    let original = wire(Bvll::OriginalBroadcastNpdu(NPDU));
    let ingress = port.receive(
        &datagram(&original, PEER, TO_BROADCAST),
        at(1_000),
        &mut Permissive,
        &mut socket,
    );
    assert!(matches!(ingress, Ingress::Npdu(delivery) if delivery.source == PEER));
    assert_eq!(
        socket.take(),
        [
            (OTHER_SITE, forwarded(GLOBAL)),
            (ROAMING, forwarded(GLOBAL)),
            (INSIDE, forwarded(GLOBAL)),
        ]
    );

    // A private foreign device's broadcast goes out from the global address;
    // a public one's keeps its own. Each is handed up from its sender.
    let distribute = wire(Bvll::DistributeBroadcastToNetwork(NPDU));
    let ingress = port.receive(
        &datagram(&distribute, INSIDE, UNICAST),
        at(1_000),
        &mut Permissive,
        &mut socket,
    );
    assert!(matches!(ingress, Ingress::Npdu(delivery) if delivery.source == INSIDE));
    assert_eq!(
        socket.take(),
        [
            (BROADCAST, forwarded(GLOBAL)),
            (OTHER_SITE, forwarded(GLOBAL)),
            (ROAMING, forwarded(GLOBAL)),
        ]
    );
    let ingress = port.receive(
        &datagram(&distribute, ROAMING, UNICAST),
        at(1_000),
        &mut Permissive,
        &mut socket,
    );
    assert!(matches!(ingress, Ingress::Npdu(delivery) if delivery.source == ROAMING));
    assert_eq!(
        socket.take(),
        [
            (BROADCAST, forwarded(ROAMING)),
            (OTHER_SITE, forwarded(ROAMING)),
            (INSIDE, forwarded(ROAMING)),
        ]
    );
}

#[test]
fn a_peers_forward_keeps_its_origin() {
    let mut tables = tables();
    let mut targets = [Target::UNUSED; Small::MAX_TARGETS];
    let mut rows = [NO_ROW; 2];
    let mut port = BbmdPort::new(link(Mode::Bbmd), &mut tables, &mut targets, &mut rows).unwrap();
    let mut socket = Socket::default();

    // Through port forwarding the peer's forward arrives from its router.
    let message = forwarded(BEHIND_OTHER);
    let ingress = port.receive(
        &datagram(&message, OTHER_SITE, UNICAST),
        at(1_000),
        &mut Permissive,
        &mut socket,
    );
    assert!(matches!(ingress, Ingress::Npdu(delivery) if delivery.source == BEHIND_OTHER));
    assert_eq!(
        socket.take(),
        [
            (BROADCAST, message.clone()),
            (ROAMING, message.clone()),
            (INSIDE, message),
        ]
    );
}
