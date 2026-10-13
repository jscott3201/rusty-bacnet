//! Properties over generated tables: the forwarding decisions against a
//! plain model written with std collections, and table invariants.

mod support;

use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{BdtEntries, BdtEntry};
use bacnet_bip_core::bbmd::{
    canonicalize, distribution_address, validate_entry, Context, Reservation, Tables, Target, Via,
    TWO_HOP_MASK,
};
use bacnet_bip_core::{Arrival, Instant, LocalSubnet};
use proptest::prelude::*;
use support::{target, OPEN};

type Site = Tables<16, 16>;

/// Addresses from a small pool, so rows, devices and sources collide often.
fn address() -> impl Strategy<Value = SocketAddrV4> {
    (0u8..4, 1u8..6, prop_oneof![Just(0xBAC0u16), Just(0xBAC1)]).prop_map(|(subnet, host, port)| {
        SocketAddrV4::new(Ipv4Addr::new(10, 0, subnet, host), port)
    })
}

const MASKS: [Ipv4Addr; 4] = [
    TWO_HOP_MASK,
    Ipv4Addr::new(255, 255, 255, 0),
    Ipv4Addr::new(255, 255, 255, 248),
    Ipv4Addr::new(255, 255, 0, 0),
];

/// Rows whose mask mostly follows their subnet, as in a real BDT, and now
/// and then doesn't, which conflicts with another row for the address.
fn rows() -> impl Strategy<Value = Vec<BdtEntry>> {
    let row = (address(), prop::sample::select(&MASKS[..]), 0u8..10).prop_map(|(a, m, odd)| {
        let usual = MASKS[usize::from(a.ip().octets()[2])];
        BdtEntry::new(a, if odd == 0 { m } else { usual })
    });
    prop::collection::vec(row, 0..10)
}

/// A registration: address, time-to-live and registration time in seconds.
fn registrations() -> impl Strategy<Value = Vec<(SocketAddrV4, u16, u64)>> {
    prop::collection::vec((address(), 1u16..120, 0u64..200), 0..12)
}

/// The model's canonical rows: valid, distinct, first seen; `None` when a
/// row is invalid or an address has two masks.
fn model_canonical(rows: &[BdtEntry]) -> Option<Vec<BdtEntry>> {
    let mut out: Vec<BdtEntry> = Vec::new();
    for row in rows {
        validate_entry(*row).ok()?;
        match out.iter().find(|seen| seen.address == row.address) {
            Some(seen) if seen.mask != row.mask => return None,
            Some(_) => {}
            None => out.push(*row),
        }
    }
    Some(out)
}

/// Build tables from generated input the way a BBMD would.
fn build(own: SocketAddrV4, rows: &[BdtEntry], regs: &[(SocketAddrV4, u16, u64)]) -> Option<Site> {
    let mut site = Site::new(own);
    site.bdt.set(BdtEntries::from_entries(rows)).ok()?;
    for (address, ttl, at) in regs {
        let _ = site.fdt.register(
            *address,
            *ttl,
            Instant::from_secs(*at),
            &OPEN,
            Reservation::Ordinary,
            || true,
        );
    }
    Some(site)
}

/// The model's broadcast targets: peers then live foreign devices, no
/// repeats, at most `fanout` devices.
fn model_targets(
    site: &Site,
    now: Instant,
    exclude: SocketAddrV4,
    fanout: usize,
) -> (Vec<SocketAddrV4>, usize, bool) {
    let own = site.bdt.own_address();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut dedup = 0;
    for row in site.bdt.rows() {
        if row.address == own || row.address == exclude {
            continue;
        }
        let dest = distribution_address(*row);
        if seen.insert(dest) {
            out.push(dest);
        } else {
            dedup += 1;
        }
    }
    let mut sent = 0;
    let mut capped = false;
    for reg in site.fdt.registrations() {
        if reg.is_expired_at(now) || reg.address == exclude {
            continue;
        }
        if !seen.insert(reg.address) {
            dedup += 1;
            continue;
        }
        if sent >= fanout {
            capped = true;
            break;
        }
        out.push(reg.address);
        sent += 1;
    }
    (out, dedup, capped)
}

proptest! {
    #[test]
    fn set_matches_the_model(own in address(), rows in rows()) {
        let mut site = Site::new(own);
        let result = site.bdt.set(BdtEntries::from_entries(&rows));
        match model_canonical(&rows) {
            None => prop_assert!(result.is_err() && site.bdt.is_empty()),
            Some(mut expected) => {
                prop_assert!(result.is_ok());
                if !expected.iter().any(|row| row.address == own) {
                    expected.push(BdtEntry::new(own, TWO_HOP_MASK));
                }
                prop_assert_eq!(site.bdt.rows(), expected.as_slice());
                let mut out = [BdtEntry::new(own, TWO_HOP_MASK); 16];
                let n = canonicalize(BdtEntries::from_entries(&rows), &mut out).unwrap();
                let distinct = model_canonical(&rows).unwrap();
                prop_assert_eq!(&out[..n], distinct.as_slice());
            }
        }
    }

    #[test]
    fn broadcast_targets_match_the_model(
        own in address(),
        rows in rows(),
        regs in registrations(),
        exclude in address(),
        now in 0u64..400,
        fanout in 0usize..6,
    ) {
        let Some(site) = build(own, &rows, &regs) else { return Ok(()) };
        let now = Instant::from_secs(now);
        let cx = Context::new(now, fanout);
        let mut out = [Target::UNUSED; Site::MAX_TARGETS];
        let got = site.broadcast_targets(&cx, exclude, &mut out);
        let (expected, dedup, capped) = model_targets(&site, now, exclude, fanout);
        let destinations: Vec<_> = got.targets.iter().map(|t| t.destination).collect();
        prop_assert_eq!(destinations, expected);
        prop_assert_eq!(got.deduplicated, dedup);
        prop_assert_eq!(got.fanout_capped, capped);
        prop_assert_eq!(got.dropped, 0);
        let peers = got.targets.iter().take_while(|t| t.via == Via::Peer).count();
        prop_assert!(got.targets[peers..].iter().all(|t| t.via == Via::ForeignDevice));
    }

    #[test]
    fn decisions_follow_the_rules(
        own in address(),
        rows in rows(),
        regs in registrations(),
        sender in address(),
        origin in address(),
        broadcast in any::<bool>(),
        now in 0u64..400,
    ) {
        let Some(site) = build(own, &rows, &regs) else { return Ok(()) };
        let now = Instant::from_secs(now);
        let local = SocketAddrV4::new(Ipv4Addr::new(10, 0, 9, 255), 0xBAC0);
        let cx = Context::new(now, 16);
        let subnet = LocalSubnet::new(local);
        let mut out = [Target::UNUSED; Site::MAX_TARGETS];
        let arrival = if broadcast { Arrival::Broadcast } else { Arrival::Unicast };
        match site.forwarded_npdu(&cx, &subnet, sender, arrival, origin, &mut out) {
            Ok(sent) => {
                prop_assert!(site.bdt.contains(sender));
                prop_assert!(sender != own);
                // J.4.5: this BBMD's own row, not the sender's, says how peers
                // reach this subnet.
                let own_row = site.bdt.row(own).unwrap();
                let local_first = sent.targets.first().map(|t| t.via) == Some(Via::LocalBroadcast);
                prop_assert_eq!(local_first, own_row.mask == TWO_HOP_MASK && !broadcast);
                let rest = &sent.targets[usize::from(local_first)..];
                prop_assert!(rest.iter().all(|t| t.via == Via::ForeignDevice && t.destination != origin));
                prop_assert!(rest.iter().all(|t| site.fdt.is_registered_at(t.destination, now)));
            }
            Err(_) => prop_assert!(sender == own || !site.bdt.contains(sender)),
        }
        let mut out = [Target::UNUSED; Site::MAX_TARGETS];
        match site.distribute_broadcast(&cx, &subnet, sender, &[1], &mut out) {
            Ok(sent) => {
                prop_assert!(site.fdt.is_registered_at(sender, now));
                prop_assert_eq!(sent.fanout.targets[0], target(local, Via::LocalBroadcast));
                prop_assert!(sent.fanout.targets[1..].iter().all(|t| t.destination != sender || t.via == Via::Peer));
            }
            Err(_) => prop_assert!(!site.fdt.is_registered_at(sender, now)),
        }
    }

    #[test]
    fn the_table_never_holds_two_registrations_for_one_address(regs in registrations(), purge_at in 0u64..400) {
        let Some(mut site) = build(SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 1), 0xBAC0), &[], &regs) else { return Ok(()) };
        let mut seen = HashSet::new();
        prop_assert!(site.fdt.registrations().iter().all(|r| seen.insert(r.address)));
        let now = Instant::from_secs(purge_at);
        let live = site.fdt.live_at(now).count();
        let purged = site.fdt.purge(now);
        prop_assert_eq!(site.fdt.len(), live);
        prop_assert_eq!(seen.len(), live + purged);
    }
}
