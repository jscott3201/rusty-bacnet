//! The NAT layout of Annex J.7.5 (Figure J-7): two sites, each behind a NAT
//! router that forwards one public port to a BBMD/router on a private
//! address. Each BDT lists the two sites' public addresses, with masks of
//! all ones, since one-hop distribution can't cross a NAT router.
//!
//! The site of BBMD 1: NAT A at 201.1.1.1, forwarding port 47809 to the
//! BBMD at 192.168.1.10:47809. The site of BBMD 2: NAT B, the same forward
//! to its own private 192.168.1.10:47809. The example gives NAT B the
//! address 237.2.2.2, which lies in the IPv4 multicast block, so the BDT
//! policy refuses it (`the_examples_second_public_address_is_multicast`);
//! these tests put NAT B at 203.0.113.2, a documentation address, instead.

mod support;

use core::net::SocketAddrV4;

use bacnet_bip_codec::{BdtEntries, BdtEntry};
use bacnet_bip_core::bbmd::{
    BdtError, Context, EntryError, ForwardRefusal, Reservation, Tables, Target, Via, TWO_HOP_MASK,
};
use bacnet_bip_core::{Arrival, Instant, LocalSubnet};
use support::{addr, destinations, target};

const NAT_PORT: u16 = 47809;
const NAT_A: SocketAddrV4 = addr(201, 1, 1, 1, NAT_PORT);
const NAT_B: SocketAddrV4 = addr(203, 0, 113, 2, NAT_PORT);
/// Both BBMDs' private address on their B/IP network 1 side.
const PRIVATE: SocketAddrV4 = addr(192, 168, 1, 10, NAT_PORT);
/// A device behind NAT B, on network 1 there.
const BEHIND_B: SocketAddrV4 = addr(192, 168, 1, 20, NAT_PORT);
/// A device behind NAT A.
const BEHIND_A: SocketAddrV4 = addr(192, 168, 1, 30, NAT_PORT);
/// The private side's broadcast address.
const PRIVATE_BROADCAST: SocketAddrV4 = addr(192, 168, 1, 255, NAT_PORT);
/// A foreign device on the public side, registered with BBMD 1.
const ROAMING: SocketAddrV4 = addr(198, 51, 100, 7, 47808);
const NPDU: &[u8] = &[0x01, 0x00, 0x10, 0x08];
const NOW: Instant = Instant::from_secs(10);

/// Both rows of the example's BDT for network 1, the same at both sites.
const BDT: [BdtEntry; 2] = [
    BdtEntry::new(NAT_A, TWO_HOP_MASK),
    BdtEntry::new(NAT_B, TWO_HOP_MASK),
];

type Site = Tables<4, 4>;

fn site(own: SocketAddrV4) -> Site {
    let mut tables = Site::new(own);
    tables.bdt.set(BdtEntries::from_entries(&BDT)).unwrap();
    tables
}

fn cx() -> Context {
    Context::new(NOW, 4)
}

/// The private side, its mask unknown: a NAT-supporting BBMD's own address
/// is the public one, so the private mask says nothing about it.
const PRIVATE_SIDE: LocalSubnet = LocalSubnet::new(PRIVATE_BROADCAST);

#[test]
fn with_its_public_address_as_its_own_each_bbmd_sends_only_to_the_other_site() {
    let bbmd1 = site(NAT_A);
    assert_eq!(
        bbmd1.bdt.rows(),
        BDT,
        "its public row is its own: none added"
    );
    let mut out = [Target::UNUSED; Site::MAX_TARGETS];
    let sent = bbmd1.original_broadcast(&cx(), BEHIND_A, NPDU, &mut out);
    // Two-hop: the peer's public address itself, which NAT B forwards.
    assert_eq!(sent.fanout.targets, [target(NAT_B, Via::Peer)]);

    let bbmd2 = site(NAT_B);
    let sent = bbmd2.own_broadcast(&cx(), NPDU, &mut out);
    assert_eq!(sent.origin, NAT_B);
    assert_eq!(destinations(sent.fanout.targets), [NAT_A]);
}

#[test]
fn a_peer_sites_forward_arrives_from_its_nat_and_is_rebroadcast_behind_this_one() {
    let mut bbmd1 = site(NAT_A);
    let registered = bbmd1.fdt.register(
        ROAMING,
        300,
        NOW,
        &support::OPEN,
        Reservation::Ordinary,
        || true,
    );
    assert!(registered.result.is_ok());
    let mut out = [Target::UNUSED; Site::MAX_TARGETS];
    // Through port forwarding the sender is NAT B's public address, a peer.
    // Its own row is two-hop, as J.7.5 requires, so the forward came to it
    // alone and goes on to the private side's broadcast.
    let sent = bbmd1
        .forwarded_npdu(
            &cx(),
            &PRIVATE_SIDE,
            NAT_B,
            Arrival::Unicast,
            BEHIND_B,
            &mut out,
        )
        .unwrap();
    assert_eq!(
        sent.targets,
        [
            target(PRIVATE_BROADCAST, Via::LocalBroadcast),
            target(ROAMING, Via::ForeignDevice),
        ]
    );
    // The private address the peer has behind its own NAT is no peer here.
    assert_eq!(
        bbmd1.forwarded_npdu(
            &cx(),
            &PRIVATE_SIDE,
            PRIVATE,
            Arrival::Unicast,
            BEHIND_B,
            &mut out
        ),
        Err(ForwardRefusal::NotFromPeer)
    );
}

#[test]
fn a_foreign_device_on_the_public_side_reaches_both_sites() {
    let mut bbmd1 = site(NAT_A);
    let registered = bbmd1.fdt.register(
        ROAMING,
        300,
        NOW,
        &support::OPEN,
        Reservation::Ordinary,
        || true,
    );
    assert!(registered.result.is_ok());
    let mut out = [Target::UNUSED; Site::MAX_TARGETS];
    let sent = bbmd1
        .distribute_broadcast(&cx(), &PRIVATE_SIDE, ROAMING, NPDU, &mut out)
        .unwrap();
    assert_eq!(
        destinations(sent.fanout.targets),
        [PRIVATE_BROADCAST, NAT_B]
    );
}

/// The private side when this BBMD is the only device of the network there
/// (Annex J.4.5, #1668).
const ALONE_BEHIND_NAT: LocalSubnet = {
    let mut subnet = LocalSubnet::new(PRIVATE_BROADCAST);
    subnet.alone = true;
    subnet
};

/// Alone on its subnet, a BBMD sends a peer's forward to its foreign
/// devices only, and a foreign device's broadcast to the peers and the other
/// foreign devices only: never to the empty private side.
#[test]
fn a_bbmd_alone_behind_its_nat_skips_the_local_rebroadcast() {
    let mut bbmd1 = site(NAT_A);
    for device in [ROAMING, addr(198, 51, 100, 8, 47808)] {
        let registered = bbmd1.fdt.register(
            device,
            300,
            NOW,
            &support::OPEN,
            Reservation::Ordinary,
            || true,
        );
        assert!(registered.result.is_ok());
    }
    let mut out = [Target::UNUSED; Site::MAX_TARGETS];
    let forwarded = bbmd1
        .forwarded_npdu(
            &cx(),
            &ALONE_BEHIND_NAT,
            NAT_B,
            Arrival::Unicast,
            BEHIND_B,
            &mut out,
        )
        .unwrap();
    assert_eq!(
        destinations(forwarded.targets),
        [ROAMING, addr(198, 51, 100, 8, 47808)]
    );

    let distributed = bbmd1
        .distribute_broadcast(&cx(), &ALONE_BEHIND_NAT, ROAMING, NPDU, &mut out)
        .unwrap();
    assert_eq!(
        distributed.fanout.targets,
        [
            target(NAT_B, Via::Peer),
            target(addr(198, 51, 100, 8, 47808), Via::ForeignDevice),
        ]
    );
}

/// Without [`Tables::set_nat`] a BBMD runs under the address it is given.
/// Given its private socket address, the public row reads as a peer, so it
/// also sends to its own NAT, and a second, private own row is added: what
/// the NAT setting (`tests/nat_rules.rs`) is for.
#[test]
fn with_its_private_address_as_its_own_a_bbmd_also_sends_to_its_own_nat() {
    let bbmd1 = site(PRIVATE);
    assert_eq!(
        bbmd1.bdt.rows(),
        [BDT[0], BDT[1], BdtEntry::new(PRIVATE, TWO_HOP_MASK)]
    );
    let mut out = [Target::UNUSED; Site::MAX_TARGETS];
    let sent = bbmd1.original_broadcast(&cx(), BEHIND_A, NPDU, &mut out);
    assert_eq!(destinations(sent.fanout.targets), [NAT_A, NAT_B]);
}

#[test]
fn the_examples_second_public_address_is_multicast() {
    let printed = BdtEntry::new(addr(237, 2, 2, 2, NAT_PORT), TWO_HOP_MASK);
    let mut tables = Site::new(NAT_A);
    assert_eq!(
        tables.bdt.set(BdtEntries::from_entries(&[BDT[0], printed])),
        Err(BdtError::Invalid {
            entry: printed,
            reason: EntryError::MulticastAddress
        })
    );
}
