//! A BBMD set up behind a NAT router ([`Tables::set_nat`], Annex J.7.5):
//! its own BDT row found by the global address, the BDT rules NAT adds,
//! the originating address of each Forwarded-NPDU it sends, and the
//! alone-on-subnet skip alongside.
//!
//! The layout is `tests/nat.rs`'s (Figure J-7, NAT B moved to a
//! documentation address): two sites whose BBMDs both sit at
//! 192.168.1.10:47809 behind their routers' public addresses.

mod support;

use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{BdtEntries, BdtEntry};
use bacnet_bip_core::bbmd::{
    BdtError, Context, EntryError, MAX_PRIVATE_RANGES, Nat, NatError, PrivateRange, Reservation,
    TWO_HOP_MASK, Tables, Target, Via, is_rfc1918,
};
use bacnet_bip_core::{Arrival, Instant, LocalSubnet};
use support::{MASK_24, addr, destinations, target};

const NAT_PORT: u16 = 47809;
const NAT_A: SocketAddrV4 = addr(201, 1, 1, 1, NAT_PORT);
const NAT_B: SocketAddrV4 = addr(203, 0, 113, 2, NAT_PORT);
/// BBMD 1's own address on its private side.
const PRIVATE: SocketAddrV4 = addr(192, 168, 1, 10, NAT_PORT);
/// A device behind NAT A.
const BEHIND_A: SocketAddrV4 = addr(192, 168, 1, 30, NAT_PORT);
/// A device behind NAT B.
const BEHIND_B: SocketAddrV4 = addr(192, 168, 1, 20, NAT_PORT);
const PRIVATE_BROADCAST: SocketAddrV4 = addr(192, 168, 1, 255, NAT_PORT);
/// A foreign device that registered from a public address.
const ROAMING: SocketAddrV4 = addr(198, 51, 100, 7, 47808);
/// A foreign device that registered from BBMD 1's private side.
const INSIDE: SocketAddrV4 = addr(192, 168, 1, 40, 47808);
/// A foreign device in a carrier-grade NAT range, private only when the
/// installation says so.
const CARRIER: SocketAddrV4 = addr(100, 64, 0, 5, 47808);
const NPDU: &[u8] = &[0x01, 0x00, 0x10, 0x08];
const NOW: Instant = Instant::from_secs(10);

const BDT: [BdtEntry; 2] = [
    BdtEntry::new(NAT_A, TWO_HOP_MASK),
    BdtEntry::new(NAT_B, TWO_HOP_MASK),
];

type Site = Tables<4, 4>;

fn cx() -> Context {
    Context::new(NOW, 4)
}

fn set(tables: &mut Site, rows: &[BdtEntry]) -> Result<(), BdtError> {
    tables.bdt.set(BdtEntries::from_entries(rows))
}

/// BBMD 1, created at its private address, then put behind NAT A with
/// `nat`, then given the example's BDT.
fn bbmd1_with(nat: Nat) -> Site {
    let mut tables = Site::new(PRIVATE);
    tables.set_nat(Some(nat), PRIVATE).unwrap();
    set(&mut tables, &BDT).unwrap();
    for device in [ROAMING, INSIDE, CARRIER] {
        let registered = tables.fdt.register(
            device,
            300,
            NOW,
            &support::OPEN,
            Reservation::Ordinary,
            || true,
        );
        assert!(registered.result.is_ok());
    }
    tables
}

fn bbmd1() -> Site {
    bbmd1_with(Nat::new(NAT_A))
}

// --- The own row ---------------------------------------------------------

#[test]
fn behind_nat_the_own_row_is_the_global_address_and_none_is_added() {
    let tables = bbmd1();
    assert_eq!(tables.bdt.own_address(), NAT_A);
    assert!(tables.bdt.behind_nat());
    assert_eq!(tables.nat().map(|nat| nat.global), Some(NAT_A));
    assert_eq!(tables.bdt.rows(), BDT, "no private own row");
    let mut out = [Target::UNUSED; Site::MAX_TARGETS];
    let sent = tables.original_broadcast(&cx(), BEHIND_A, NPDU, &mut out);
    assert_eq!(
        destinations(sent.fanout.targets),
        [NAT_B, ROAMING, INSIDE, CARRIER],
        "never to its own NAT"
    );
}

#[test]
fn setting_nat_on_a_table_already_set_finds_the_own_row_there() {
    let mut tables = Site::new(PRIVATE);
    set(&mut tables, &BDT).unwrap();
    // Without NAT the private own row is appended.
    assert_eq!(tables.bdt.len(), 3);
    tables.set_nat(Some(Nat::new(NAT_A)), PRIVATE).unwrap();
    assert_eq!(tables.bdt.rows(), BDT, "the appended private row goes");
    assert_eq!(tables.bdt.own_address(), NAT_A);
}

#[test]
fn a_table_without_the_global_address_refuses_nat_and_changes_nothing() {
    let mut tables = Site::new(PRIVATE);
    let peer_only = [BdtEntry::new(NAT_B, TWO_HOP_MASK)];
    set(&mut tables, &peer_only).unwrap();
    let before = tables.bdt.rows().to_vec();
    assert_eq!(
        tables.set_nat(Some(Nat::new(NAT_A)), PRIVATE),
        Err(BdtError::NoOwnRow { own: NAT_A })
    );
    assert_eq!(tables.bdt.rows(), before);
    assert_eq!(tables.bdt.own_address(), PRIVATE);
    assert!(tables.nat().is_none());
}

#[test]
fn behind_nat_a_table_without_the_global_address_is_refused() {
    let mut tables = bbmd1();
    let peer_only = [BdtEntry::new(NAT_B, TWO_HOP_MASK)];
    assert_eq!(
        set(&mut tables, &peer_only),
        Err(BdtError::NoOwnRow { own: NAT_A })
    );
    assert_eq!(tables.bdt.rows(), BDT, "unchanged");
}

/// A BBMD behind NAT with no peers serves only foreign devices (Annex
/// J.7.5): an empty table is taken, and no own row is added to it.
#[test]
fn behind_nat_an_empty_table_serves_foreign_devices_only() {
    let mut tables = bbmd1();
    set(&mut tables, &[]).unwrap();
    assert!(tables.bdt.is_empty(), "the global row is never added");
    let mut out = [Target::UNUSED; Site::MAX_TARGETS];
    let sent = tables.own_broadcast(&cx(), NPDU, &mut out);
    assert_eq!(sent.origin, NAT_A);
    assert_eq!(
        destinations(sent.fanout.targets),
        [ROAMING, INSIDE, CARRIER]
    );
    // Turning NAT on over an empty table, and off again, works too.
    let mut fresh = Site::new(PRIVATE);
    fresh.set_nat(Some(Nat::new(NAT_A)), PRIVATE).unwrap();
    set(&mut fresh, &[]).unwrap();
    fresh.set_nat(None, PRIVATE).unwrap();
    assert_eq!(fresh.bdt.rows(), [BdtEntry::new(PRIVATE, TWO_HOP_MASK)]);
}

#[test]
fn behind_nat_a_one_hop_row_is_refused() {
    let mut tables = bbmd1();
    let one_hop = BdtEntry::new(addr(198, 51, 100, 1, NAT_PORT), MASK_24);
    assert_eq!(
        set(&mut tables, &[BDT[0], BDT[1], one_hop]),
        Err(BdtError::OneHopBehindNat { entry: one_hop })
    );
    // The own row too.
    let own_one_hop = BdtEntry::new(NAT_A, MASK_24);
    assert_eq!(
        set(&mut tables, &[own_one_hop, BDT[1]]),
        Err(BdtError::OneHopBehindNat { entry: own_one_hop })
    );
    assert_eq!(tables.bdt.rows(), BDT, "unchanged");

    // Turning NAT on over a one-hop table is refused the same way.
    let mut plain = Site::new(PRIVATE);
    set(&mut plain, &[BDT[0], one_hop]).unwrap();
    assert_eq!(
        plain.set_nat(Some(Nat::new(NAT_A)), PRIVATE),
        Err(BdtError::OneHopBehindNat { entry: one_hop })
    );
    assert!(plain.nat().is_none());
}

#[test]
fn the_ordinary_row_rules_still_apply_behind_nat() {
    let mut tables = bbmd1();
    let multicast = BdtEntry::new(addr(237, 2, 2, 2, NAT_PORT), TWO_HOP_MASK);
    assert_eq!(
        set(&mut tables, &[BDT[0], multicast]),
        Err(BdtError::Invalid {
            entry: multicast,
            reason: EntryError::MulticastAddress
        })
    );
}

#[test]
fn a_global_address_no_peer_could_send_to_is_refused() {
    let mut tables = Site::new(PRIVATE);
    for (global, reason) in [
        (addr(201, 1, 1, 1, 0), EntryError::ZeroPort),
        (addr(0, 0, 0, 0, NAT_PORT), EntryError::UnspecifiedAddress),
        (addr(239, 1, 1, 1, NAT_PORT), EntryError::MulticastAddress),
    ] {
        assert_eq!(
            tables.set_nat(Some(Nat::new(global)), PRIVATE),
            Err(BdtError::Invalid {
                entry: BdtEntry::new(global, TWO_HOP_MASK),
                reason
            })
        );
    }
    assert!(tables.nat().is_none());
}

#[test]
fn turning_nat_off_returns_to_the_local_address_and_drops_the_global_row() {
    let mut tables = bbmd1();
    tables.set_nat(None, PRIVATE).unwrap();
    assert!(!tables.bdt.behind_nat());
    assert_eq!(tables.bdt.own_address(), PRIVATE);
    // The old global row named this BBMD; it would read as a peer now.
    assert_eq!(
        tables.bdt.rows(),
        [BDT[1], BdtEntry::new(PRIVATE, TWO_HOP_MASK)]
    );
    // set_nat_and_bdt drops it from the table it is given, too.
    let mut again = bbmd1();
    again
        .set_nat_and_bdt(None, PRIVATE, BdtEntries::from_entries(&BDT))
        .unwrap();
    assert_eq!(
        again.bdt.rows(),
        [BDT[1], BdtEntry::new(PRIVATE, TWO_HOP_MASK)]
    );
    // One-hop rows are allowed again.
    let one_hop = BdtEntry::new(addr(198, 51, 100, 1, NAT_PORT), MASK_24);
    set(&mut tables, &[one_hop]).unwrap();
}

#[test]
fn a_new_global_address_must_be_listed_too() {
    let mut tables = bbmd1();
    // The router's address changes: the table must name the new one.
    let moved = addr(201, 1, 1, 2, NAT_PORT);
    assert_eq!(
        tables.set_nat(Some(Nat::new(moved)), PRIVATE),
        Err(BdtError::NoOwnRow { own: moved })
    );
    assert_eq!(tables.bdt.own_address(), NAT_A);
    assert_eq!(
        tables.bdt.set_own_address(moved),
        Err(BdtError::NoOwnRow { own: moved })
    );
    let listed = [BdtEntry::new(moved, TWO_HOP_MASK), BDT[1]];
    set(&mut tables, &listed).unwrap_err();
    // Listed together with the old row, it can move.
    let both = [BDT[0], BdtEntry::new(moved, TWO_HOP_MASK), BDT[1]];
    set(&mut tables, &both).unwrap();
    tables.set_nat(Some(Nat::new(moved)), PRIVATE).unwrap();
    assert_eq!(tables.bdt.own_address(), moved);
    assert_eq!(tables.bdt.rows(), both, "the old row stays, as a peer");
}

// --- Originating addresses ------------------------------------------------

#[test]
fn a_broadcast_from_the_private_side_goes_out_from_the_global_address() {
    let tables = bbmd1();
    let mut out = [Target::UNUSED; Site::MAX_TARGETS];
    let sent = tables.original_broadcast(&cx(), BEHIND_A, NPDU, &mut out);
    assert_eq!(sent.origin, NAT_A);
    assert!(sent.transmissions().all(|(_, message)| message
        == bacnet_bip_codec::Bvll::ForwardedNpdu {
            origin: NAT_A,
            npdu: NPDU
        }));
}

#[test]
fn the_bbmds_own_broadcast_goes_out_from_the_global_address() {
    let tables = bbmd1();
    let mut out = [Target::UNUSED; Site::MAX_TARGETS];
    let sent = tables.own_broadcast(&cx(), NPDU, &mut out);
    assert_eq!(sent.origin, NAT_A);
    assert_eq!(
        destinations(sent.fanout.targets),
        [NAT_B, ROAMING, INSIDE, CARRIER]
    );
}

#[test]
fn a_private_foreign_devices_broadcast_goes_out_from_the_global_address() {
    let tables = bbmd1();
    let mut out = [Target::UNUSED; Site::MAX_TARGETS];
    let subnet = LocalSubnet::new(PRIVATE_BROADCAST);
    let sent = tables
        .distribute_broadcast(&cx(), &subnet, INSIDE, NPDU, &mut out)
        .unwrap();
    assert_eq!(sent.origin, NAT_A);
    // Still never back to the device itself.
    assert_eq!(
        destinations(sent.fanout.targets),
        [PRIVATE_BROADCAST, NAT_B, ROAMING, CARRIER]
    );
}

#[test]
fn a_public_foreign_devices_broadcast_keeps_its_own_address() {
    let tables = bbmd1();
    let mut out = [Target::UNUSED; Site::MAX_TARGETS];
    let subnet = LocalSubnet::new(PRIVATE_BROADCAST);
    let sent = tables
        .distribute_broadcast(&cx(), &subnet, ROAMING, NPDU, &mut out)
        .unwrap();
    assert_eq!(sent.origin, ROAMING);
    // Not private unless the installation says so.
    let sent = tables
        .distribute_broadcast(&cx(), &subnet, CARRIER, NPDU, &mut out)
        .unwrap();
    assert_eq!(sent.origin, CARRIER);
}

#[test]
fn a_range_listed_as_private_counts_as_private() {
    let mut nat = Nat::new(NAT_A);
    let carrier = PrivateRange::new(Ipv4Addr::new(100, 64, 0, 0), 10).unwrap();
    nat.add_private_range(carrier).unwrap();
    let tables = bbmd1_with(nat);
    let mut out = [Target::UNUSED; Site::MAX_TARGETS];
    let subnet = LocalSubnet::new(PRIVATE_BROADCAST);
    let sent = tables
        .distribute_broadcast(&cx(), &subnet, CARRIER, NPDU, &mut out)
        .unwrap();
    assert_eq!(sent.origin, NAT_A);
    let sent = tables
        .distribute_broadcast(&cx(), &subnet, ROAMING, NPDU, &mut out)
        .unwrap();
    assert_eq!(sent.origin, ROAMING);
}

#[test]
fn without_nat_every_origin_is_the_source() {
    let mut tables = Site::new(PRIVATE);
    set(&mut tables, &BDT).unwrap();
    let registered = tables.fdt.register(
        INSIDE,
        300,
        NOW,
        &support::OPEN,
        Reservation::Ordinary,
        || true,
    );
    assert!(registered.result.is_ok());
    let mut out = [Target::UNUSED; Site::MAX_TARGETS];
    let subnet = LocalSubnet::new(PRIVATE_BROADCAST);
    assert_eq!(
        tables
            .original_broadcast(&cx(), BEHIND_A, NPDU, &mut out)
            .origin,
        BEHIND_A
    );
    assert_eq!(
        tables
            .distribute_broadcast(&cx(), &subnet, INSIDE, NPDU, &mut out)
            .unwrap()
            .origin,
        INSIDE
    );
}

/// A peer's Forwarded-NPDU goes on unchanged: the decision returns only
/// where, and the caller sends the origin it received
/// (`tests/port_bbmd_nat.rs` checks the wire).
#[test]
fn a_received_forward_is_passed_on_to_the_private_side_and_foreign_devices() {
    let tables = bbmd1();
    let mut out = [Target::UNUSED; Site::MAX_TARGETS];
    let subnet = LocalSubnet::new(PRIVATE_BROADCAST);
    let fanout = tables
        .forwarded_npdu(&cx(), &subnet, NAT_B, Arrival::Unicast, BEHIND_B, &mut out)
        .unwrap();
    assert_eq!(
        fanout.targets,
        [
            target(PRIVATE_BROADCAST, Via::LocalBroadcast),
            target(ROAMING, Via::ForeignDevice),
            target(INSIDE, Via::ForeignDevice),
            target(CARRIER, Via::ForeignDevice),
        ]
    );
}

// --- The subnet mask and the alone-on-subnet skip -------------------------

/// Behind NAT the own address is the global one, on no local subnet, so a
/// peer whose public address happens to share the global address's /24 is
/// still a remote peer, and its forward is broadcast behind the router.
#[test]
fn behind_nat_the_subnet_mask_doesnt_place_a_peer_on_the_subnet() {
    let neighbour = addr(201, 1, 1, 9, NAT_PORT);
    let mut tables = bbmd1();
    set(
        &mut tables,
        &[BDT[0], BDT[1], BdtEntry::new(neighbour, TWO_HOP_MASK)],
    )
    .unwrap();
    assert!(tables.rebroadcasts_forwarded(Some(MASK_24), neighbour, Arrival::Unicast));
    // Without NAT, at the same address, it would count as on the subnet.
    let mut plain = Site::new(NAT_A);
    set(
        &mut plain,
        &[BDT[0], BdtEntry::new(neighbour, TWO_HOP_MASK)],
    )
    .unwrap();
    assert!(!plain.rebroadcasts_forwarded(Some(MASK_24), neighbour, Arrival::Unicast));
}

#[test]
fn alone_behind_nat_neither_path_rebroadcasts_locally() {
    let tables = bbmd1();
    let mut out = [Target::UNUSED; Site::MAX_TARGETS];
    let mut alone = LocalSubnet::new(PRIVATE_BROADCAST);
    alone.alone = true;
    let forwarded = tables
        .forwarded_npdu(&cx(), &alone, NAT_B, Arrival::Unicast, BEHIND_B, &mut out)
        .unwrap();
    assert_eq!(destinations(forwarded.targets), [ROAMING, INSIDE, CARRIER]);
    let distributed = tables
        .distribute_broadcast(&cx(), &alone, INSIDE, NPDU, &mut out)
        .unwrap();
    assert_eq!(
        distributed.origin, NAT_A,
        "the skip leaves the origin alone"
    );
    assert_eq!(
        destinations(distributed.fanout.targets),
        [NAT_B, ROAMING, CARRIER]
    );
}

// --- The address ranges ---------------------------------------------------

#[test]
fn the_rfc1918_ranges_and_their_edges() {
    for private in [
        Ipv4Addr::new(10, 0, 0, 0),
        Ipv4Addr::new(10, 255, 255, 255),
        Ipv4Addr::new(172, 16, 0, 1),
        Ipv4Addr::new(172, 31, 255, 255),
        Ipv4Addr::new(192, 168, 0, 1),
    ] {
        assert!(is_rfc1918(private), "{private}");
    }
    for public in [
        Ipv4Addr::new(11, 0, 0, 1),
        Ipv4Addr::new(172, 15, 255, 255),
        Ipv4Addr::new(172, 32, 0, 0),
        Ipv4Addr::new(192, 169, 0, 1),
        Ipv4Addr::new(100, 64, 0, 1),
    ] {
        assert!(!is_rfc1918(public), "{public}");
    }
}

#[test]
fn private_ranges_clear_host_bits_and_hold_a_few() {
    assert_eq!(
        PrivateRange::new(Ipv4Addr::new(10, 0, 0, 0), 33),
        Err(NatError::Prefix)
    );
    let range = PrivateRange::new(Ipv4Addr::new(100, 127, 1, 2), 10).unwrap();
    assert_eq!(range.network(), Ipv4Addr::new(100, 64, 0, 0));
    assert_eq!(range.prefix(), 10);
    assert_eq!(range.to_string(), "100.64.0.0/10");
    assert!(range.contains(Ipv4Addr::new(100, 64, 0, 1)));
    assert!(!range.contains(Ipv4Addr::new(100, 128, 0, 1)));
    let everything = PrivateRange::new(Ipv4Addr::new(1, 2, 3, 4), 0).unwrap();
    assert!(everything.contains(Ipv4Addr::new(8, 8, 8, 8)));

    let mut nat = Nat::new(NAT_A);
    for i in 0..MAX_PRIVATE_RANGES {
        let octet = u8::try_from(i).unwrap();
        let range = PrivateRange::new(Ipv4Addr::new(100, 64, octet, 0), 24).unwrap();
        nat.add_private_range(range).unwrap();
        // Adding one twice keeps one copy.
        nat.add_private_range(range).unwrap();
    }
    assert_eq!(nat.private_ranges().len(), MAX_PRIVATE_RANGES);
    let one_more = PrivateRange::new(Ipv4Addr::new(100, 65, 0, 0), 16).unwrap();
    assert_eq!(
        nat.add_private_range(one_more),
        Err(NatError::TooManyRanges)
    );
    assert!(nat.is_private(Ipv4Addr::new(100, 64, 3, 9)));
    assert!(nat.is_private(Ipv4Addr::new(192, 168, 9, 9)));
    assert!(!nat.is_private(Ipv4Addr::new(100, 65, 0, 1)));
}

#[test]
fn a_new_global_address_and_its_table_go_in_together() {
    let mut tables = bbmd1();
    let moved = addr(201, 1, 1, 2, NAT_PORT);
    let table = [BdtEntry::new(moved, TWO_HOP_MASK), BDT[1]];
    // Refused alone in either order, taken in one step.
    tables
        .set_nat_and_bdt(
            Some(Nat::new(moved)),
            PRIVATE,
            BdtEntries::from_entries(&table),
        )
        .unwrap();
    assert_eq!(tables.bdt.own_address(), moved);
    assert_eq!(tables.bdt.rows(), table);
    // A table the new setting refuses changes nothing.
    assert_eq!(
        tables.set_nat_and_bdt(
            Some(Nat::new(NAT_A)),
            PRIVATE,
            BdtEntries::from_entries(&table)
        ),
        Err(BdtError::NoOwnRow { own: NAT_A })
    );
    assert_eq!(tables.bdt.own_address(), moved);
    // Off, with a one-hop table: the local row is appended.
    let one_hop = BdtEntry::new(addr(198, 51, 100, 1, NAT_PORT), MASK_24);
    tables
        .set_nat_and_bdt(None, PRIVATE, BdtEntries::from_entries(&[one_hop]))
        .unwrap();
    assert!(tables.nat().is_none());
    assert_eq!(
        tables.bdt.rows(),
        [one_hop, BdtEntry::new(PRIVATE, TWO_HOP_MASK)]
    );
}
