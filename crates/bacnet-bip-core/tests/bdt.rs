//! The broadcast distribution table: validation, canonical form, the own row
//! and capacity (Annex J.4.3).

mod support;

use core::net::Ipv4Addr;

use bacnet_bip_codec::{BdtEntries, BdtEntry};
use bacnet_bip_core::bbmd::{
    canonicalize, distribution_address, validate_entry, Bdt, BdtError, EntryError, TWO_HOP_MASK,
};
use support::{addr, row, MASK_24, MASK_32, OWN, PORT};

fn set<const N: usize>(bdt: &mut Bdt<N>, rows: &[BdtEntry]) -> Result<(), BdtError> {
    bdt.set(BdtEntries::from_entries(rows))
}

#[test]
fn a_new_table_is_empty_until_set() {
    let bdt: Bdt<4> = Bdt::new(OWN);
    assert!(bdt.is_empty());
    assert_eq!(bdt.len(), 0);
    assert_eq!(bdt.own_address(), OWN);
    assert!(bdt.entries().is_empty());
    assert!(!bdt.contains(OWN));
    assert_eq!(Bdt::<4>::CAPACITY, 4);
}

#[test]
fn setting_an_empty_table_gives_the_own_row_alone() {
    let mut bdt: Bdt<4> = Bdt::new(OWN);
    set(&mut bdt, &[]).unwrap();
    assert_eq!(bdt.rows(), [row(OWN, TWO_HOP_MASK)]);
    assert!(bdt.configured_rows().is_empty());
    assert!(bdt.contains(OWN));
}

#[test]
fn the_own_row_is_appended_after_the_peers_and_read_through_the_codec_view() {
    let mut bdt: Bdt<4> = Bdt::new(OWN);
    let peer = row(addr(10, 0, 2, 1, PORT), MASK_24);
    set(&mut bdt, &[peer]).unwrap();
    assert_eq!(bdt.rows(), [peer, row(OWN, MASK_32)]);
    assert_eq!(bdt.configured_rows(), [peer]);
    let view: Vec<BdtEntry> = bdt.entries().iter().collect();
    assert_eq!(view, bdt.rows());
    assert_eq!(bdt.row(peer.address), Some(peer));
    assert_eq!(bdt.row(addr(10, 0, 2, 2, PORT)), None);
}

#[test]
fn a_listed_own_row_keeps_its_place_and_mask() {
    let mut bdt: Bdt<4> = Bdt::new(OWN);
    let own = row(OWN, MASK_24);
    let peer = row(addr(10, 0, 2, 1, PORT), MASK_24);
    set(&mut bdt, &[own, peer]).unwrap();
    assert_eq!(bdt.rows(), [own, peer]);
    assert_eq!(bdt.configured_rows(), [own, peer]);
}

#[test]
fn identical_rows_collapse_in_first_seen_order() {
    let mut bdt: Bdt<4> = Bdt::new(OWN);
    let a = row(addr(10, 0, 0, 5, PORT), MASK_32);
    let b = row(addr(10, 0, 0, 6, PORT), MASK_32);
    set(&mut bdt, &[a, b, a, b, a]).unwrap();
    assert_eq!(bdt.rows(), [a, b, row(OWN, MASK_32)]);
}

#[test]
fn one_address_with_two_masks_refuses_the_table_and_keeps_the_old_one() {
    let mut bdt: Bdt<4> = Bdt::new(OWN);
    let committed = row(addr(10, 0, 0, 5, PORT), MASK_32);
    set(&mut bdt, &[committed]).unwrap();
    let before = bdt.rows().to_vec();
    let later = row(addr(10, 0, 0, 9, PORT), MASK_24);
    let err = set(&mut bdt, &[row(later.address, MASK_32), later]).unwrap_err();
    assert_eq!(err, BdtError::ConflictingMasks { entry: later });
    assert_eq!(bdt.rows(), before);
}

#[test]
fn the_same_ip_at_two_ports_is_two_peers() {
    let mut bdt: Bdt<4> = Bdt::new(OWN);
    let a = row(addr(10, 0, 0, 5, PORT), MASK_32);
    let b = row(addr(10, 0, 0, 5, PORT + 1), MASK_32);
    set(&mut bdt, &[a, b]).unwrap();
    assert!(bdt.contains(a.address) && bdt.contains(b.address));
    assert_eq!(bdt.len(), 3);
}

#[test]
fn capacity_counts_the_own_row_and_refuses_without_change() {
    let mut bdt: Bdt<3> = Bdt::new(OWN);
    let peers = [
        row(addr(10, 0, 0, 1, PORT), MASK_32),
        row(addr(10, 0, 0, 2, PORT), MASK_32),
        row(addr(10, 0, 0, 3, PORT), MASK_32),
    ];
    // Two peers and the own row fit exactly.
    set(&mut bdt, &peers[..2]).unwrap();
    assert_eq!(bdt.len(), 3);
    let before = bdt.rows().to_vec();
    // Three peers would need a fourth row for the own address.
    assert_eq!(
        set(&mut bdt, &peers),
        Err(BdtError::Capacity {
            needed: 4,
            available: 3
        })
    );
    assert_eq!(bdt.rows(), before);
    // Three rows, one of them the own: they fit.
    set(&mut bdt, &[peers[0], peers[1], row(OWN, MASK_32)]).unwrap();
    assert_eq!(bdt.len(), 3);
}

#[test]
fn duplicates_collapse_before_capacity_is_counted() {
    let mut bdt: Bdt<3> = Bdt::new(OWN);
    let a = row(addr(10, 0, 0, 1, PORT), MASK_32);
    let b = row(addr(10, 0, 0, 2, PORT), MASK_32);
    set(&mut bdt, &[a, b, a, b, b, a]).unwrap();
    assert_eq!(bdt.rows(), [a, b, row(OWN, MASK_32)]);
}

#[test]
fn a_bad_row_past_the_capacity_is_reported_before_the_capacity() {
    let mut bdt: Bdt<2> = Bdt::new(OWN);
    let rows = [
        row(addr(10, 0, 0, 1, PORT), MASK_32),
        row(addr(10, 0, 0, 2, PORT), MASK_32),
        row(addr(10, 0, 0, 3, PORT), MASK_32),
        row(addr(10, 0, 0, 3, PORT), MASK_24),
    ];
    assert_eq!(
        set(&mut bdt, &rows),
        Err(BdtError::ConflictingMasks { entry: rows[3] })
    );
    let bad = row(addr(224, 0, 0, 1, PORT), MASK_32);
    assert_eq!(
        set(&mut bdt, &[rows[0], rows[1], rows[2], bad]),
        Err(BdtError::Invalid {
            entry: bad,
            reason: EntryError::MulticastAddress
        })
    );
    // Duplicates of rows that found no room still collapse.
    assert_eq!(
        set(&mut bdt, &[rows[0], rows[1], rows[2], rows[2], rows[1]]),
        Err(BdtError::Capacity {
            needed: 4,
            available: 2
        })
    );
}

/// Every distinct row past the storage counts toward `needed`, not just the
/// first, and a duplicate of one of them doesn't.
#[test]
fn every_distinct_row_past_the_storage_is_counted() {
    let rows = [
        row(addr(10, 0, 0, 1, PORT), MASK_32),
        row(addr(10, 0, 0, 2, PORT), MASK_32),
        row(addr(10, 0, 0, 3, PORT), MASK_32),
        row(addr(10, 0, 0, 4, PORT), MASK_32),
        row(addr(10, 0, 0, 5, PORT), MASK_32),
        row(addr(10, 0, 0, 4, PORT), MASK_32),
    ];
    let mut bdt: Bdt<2> = Bdt::new(OWN);
    assert_eq!(
        set(&mut bdt, &rows),
        Err(BdtError::Capacity {
            needed: 6,
            available: 2
        })
    );
    let mut out = [row(addr(0, 0, 0, 0, 0), MASK_32); 2];
    assert_eq!(
        canonicalize(BdtEntries::from_entries(&rows), &mut out),
        Err(BdtError::Capacity {
            needed: 5,
            available: 2
        })
    );
}

#[test]
fn canonicalize_writes_distinct_rows_without_an_own_row() {
    let a = row(addr(10, 0, 0, 1, PORT), MASK_32);
    let b = row(addr(10, 0, 1, 1, PORT), MASK_24);
    let mut out = [row(addr(0, 0, 0, 0, 0), MASK_32); 4];
    let n = canonicalize(BdtEntries::from_entries(&[a, b, a]), &mut out).unwrap();
    assert_eq!(&out[..n], [a, b]);
    let mut small = [row(addr(0, 0, 0, 0, 0), MASK_32); 1];
    assert_eq!(
        canonicalize(BdtEntries::from_entries(&[a, b, a]), &mut small),
        Err(BdtError::Capacity {
            needed: 2,
            available: 1
        })
    );
    let wire = [10, 0, 0, 1, 0xBA, 0xC0, 255, 255, 255, 255];
    let n = canonicalize(BdtEntries::from_wire(&wire).unwrap(), &mut out).unwrap();
    assert_eq!(&out[..n], [a]);
}

#[test]
fn each_validation_rule_refuses_its_row() {
    let cases = [
        (row(addr(10, 0, 0, 6, 0), MASK_32), EntryError::ZeroPort),
        (
            row(addr(0, 0, 0, 0, PORT), MASK_32),
            EntryError::UnspecifiedAddress,
        ),
        (
            row(addr(255, 255, 255, 255, PORT), MASK_32),
            EntryError::LimitedBroadcastAddress,
        ),
        (
            row(addr(224, 0, 0, 1, PORT), MASK_32),
            EntryError::MulticastAddress,
        ),
        (
            row(addr(239, 255, 255, 250, PORT), MASK_32),
            EntryError::MulticastAddress,
        ),
        (
            row(addr(10, 0, 0, 5, PORT), Ipv4Addr::new(255, 0, 255, 0)),
            EntryError::NonContiguousMask,
        ),
        (
            row(addr(10, 0, 0, 5, PORT), Ipv4Addr::new(254, 0, 0, 0)),
            EntryError::MaskTooWide,
        ),
        (
            row(addr(10, 0, 0, 5, PORT), Ipv4Addr::UNSPECIFIED),
            EntryError::MaskTooWide,
        ),
        (
            row(addr(192, 168, 1, 0, PORT), MASK_24),
            EntryError::NetworkAddress,
        ),
        (
            row(addr(192, 168, 1, 255, PORT), MASK_24),
            EntryError::DirectedBroadcastAddress,
        ),
        (
            row(addr(255, 0, 0, 1, PORT), Ipv4Addr::new(255, 0, 0, 0)),
            EntryError::LimitedBroadcastTarget,
        ),
    ];
    for (entry, reason) in cases {
        assert_eq!(validate_entry(entry), Err(reason), "{entry:?}");
        let mut bdt: Bdt<4> = Bdt::new(OWN);
        assert_eq!(
            set(&mut bdt, &[entry]),
            Err(BdtError::Invalid { entry, reason })
        );
        assert!(bdt.is_empty());
        assert!(!reason.reason().is_empty());
        assert_eq!(reason.to_string(), reason.reason());
    }
}

#[test]
fn ordinary_rows_pass_validation() {
    for entry in [
        row(addr(10, 0, 0, 5, PORT), MASK_32),
        row(addr(192, 168, 2, 10, PORT), MASK_24),
        row(addr(127, 0, 0, 2, PORT), MASK_32),
        row(addr(169, 254, 10, 20, PORT), MASK_32),
        row(addr(10, 0, 0, 1, PORT), Ipv4Addr::new(255, 0, 0, 0)),
        row(addr(223, 255, 255, 1, PORT), MASK_24),
        row(addr(240, 0, 0, 1, PORT), MASK_32),
    ] {
        assert_eq!(validate_entry(entry), Ok(()), "{entry:?}");
    }
}

#[test]
fn errors_display_what_went_wrong() {
    let entry = row(addr(10, 0, 0, 6, 0), MASK_32);
    let invalid = BdtError::Invalid {
        entry,
        reason: EntryError::ZeroPort,
    };
    assert!(invalid.to_string().contains("10.0.0.6:0"));
    assert!(invalid.to_string().contains("invalid port 0"));
    assert!(BdtError::ConflictingMasks { entry }
        .to_string()
        .contains("conflicting masks"));
    assert!(BdtError::Capacity {
        needed: 5,
        available: 4
    }
    .to_string()
    .contains('5'));
}

#[test]
fn the_mask_picks_the_destination() {
    let peer = addr(10, 1, 2, 3, PORT);
    for (mask, expected) in [
        (MASK_32, addr(10, 1, 2, 3, PORT)),
        (MASK_24, addr(10, 1, 2, 255, PORT)),
        (Ipv4Addr::new(255, 255, 0, 0), addr(10, 1, 255, 255, PORT)),
        (Ipv4Addr::new(255, 0, 0, 0), addr(10, 255, 255, 255, PORT)),
        (Ipv4Addr::new(255, 255, 255, 252), addr(10, 1, 2, 3, PORT)),
        (Ipv4Addr::new(255, 255, 255, 248), addr(10, 1, 2, 7, PORT)),
    ] {
        assert_eq!(distribution_address(row(peer, mask)), expected, "{mask}");
    }
}

#[test]
fn moving_the_own_address_moves_an_appended_own_row() {
    let mut bdt: Bdt<4> = Bdt::new(OWN);
    let peer = row(addr(10, 0, 2, 1, PORT), MASK_32);
    set(&mut bdt, &[peer]).unwrap();
    let moved = addr(192, 168, 1, 2, PORT);
    bdt.set_own_address(moved).unwrap();
    assert_eq!(bdt.own_address(), moved);
    assert_eq!(bdt.rows(), [peer, row(moved, MASK_32)]);
    assert_eq!(bdt.configured_rows(), [peer]);
    // The same address again changes nothing.
    bdt.set_own_address(moved).unwrap();
    assert_eq!(bdt.rows(), [peer, row(moved, MASK_32)]);
}

#[test]
fn moving_the_own_address_keeps_a_listed_row_as_a_peer() {
    let mut bdt: Bdt<4> = Bdt::new(OWN);
    let listed = row(OWN, MASK_32);
    set(&mut bdt, &[listed]).unwrap();
    let moved = addr(192, 168, 1, 2, PORT);
    bdt.set_own_address(moved).unwrap();
    assert_eq!(bdt.rows(), [listed, row(moved, MASK_32)]);
    // Back again: the listed row is the own row once more.
    bdt.set_own_address(OWN).unwrap();
    assert_eq!(bdt.rows(), [listed]);
    assert_eq!(bdt.configured_rows(), [listed]);
}

#[test]
fn moving_the_own_address_of_an_unset_table_adds_its_row() {
    let mut bdt: Bdt<2> = Bdt::new(OWN);
    let moved = addr(192, 168, 1, 2, PORT);
    bdt.set_own_address(moved).unwrap();
    assert_eq!(bdt.rows(), [row(moved, MASK_32)]);
}

#[test]
fn moving_the_own_address_refuses_to_overflow() {
    let mut bdt: Bdt<2> = Bdt::new(OWN);
    let rows = [row(addr(10, 0, 0, 1, PORT), MASK_32), row(OWN, MASK_32)];
    set(&mut bdt, &rows).unwrap();
    assert_eq!(
        bdt.set_own_address(addr(192, 168, 1, 2, PORT)),
        Err(BdtError::Capacity {
            needed: 3,
            available: 2
        })
    );
    assert_eq!(bdt.own_address(), OWN);
    assert_eq!(bdt.rows(), rows);
}

#[test]
fn debug_shows_only_the_rows_in_use() {
    let mut bdt: Bdt<64> = Bdt::new(OWN);
    set(&mut bdt, &[]).unwrap();
    let text = format!("{bdt:?}");
    assert!(text.contains("192.168.1.1:47808"), "{text}");
    assert!(!text.contains("0.0.0.0"), "{text}");
}
