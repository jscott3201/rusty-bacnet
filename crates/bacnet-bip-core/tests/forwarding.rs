//! Where each broadcast goes (Annex J.4.5): one-hop and two-hop peers,
//! foreign devices, the local subnet, deduplication and the fan-out budget.

mod support;

use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::Bvll;
use bacnet_bip_core::bbmd::{Context, DistributeRefusal, ForwardRefusal, Tables, Target, Via};
use bacnet_bip_core::{Arrival, Instant};
use support::{
    addr, context, destinations, register_all, row, secs, storage, tables, target, Small,
    LOCAL_BROADCAST, MASK_24, MASK_32, OWN, PORT, SUBNET,
};

const NPDU: &[u8] = &[0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];
const T0: Instant = Instant::from_secs(500);

/// A device on this BBMD's subnet.
const LOCAL_DEVICE: SocketAddrV4 = addr(192, 168, 1, 100, PORT);
/// A two-hop peer and a one-hop peer.
const TWO_HOP: SocketAddrV4 = addr(10, 0, 0, 1, PORT);
const ONE_HOP: SocketAddrV4 = addr(192, 168, 2, 1, PORT);
/// Two foreign devices.
const FD1: SocketAddrV4 = addr(172, 16, 0, 5, PORT);
const FD2: SocketAddrV4 = addr(172, 16, 0, 6, 47809);

/// The BBMD with both peers and both foreign devices.
fn site() -> Small {
    let mut tables = tables(&[row(TWO_HOP, MASK_32), row(ONE_HOP, MASK_24)]);
    register_all(&mut tables, &[FD1, FD2], 60, T0);
    tables
}

#[test]
fn an_original_broadcast_goes_to_every_peer_subnet_and_foreign_device() {
    let tables = site();
    let mut out = storage();
    let sent = tables.original_broadcast(&context(T0), LOCAL_DEVICE, NPDU, &mut out);
    assert_eq!(sent.origin, LOCAL_DEVICE);
    assert_eq!(sent.npdu, NPDU);
    assert_eq!(
        sent.message(),
        Bvll::ForwardedNpdu {
            origin: LOCAL_DEVICE,
            npdu: NPDU
        }
    );
    assert_eq!(
        sent.fanout.targets,
        [
            target(TWO_HOP, Via::Peer),
            target(addr(192, 168, 2, 255, PORT), Via::Peer),
            target(FD1, Via::ForeignDevice),
            target(FD2, Via::ForeignDevice),
        ]
    );
    assert_eq!(sent.fanout.deduplicated, 0);
    assert!(!sent.fanout.fanout_capped);
    assert_eq!(sent.fanout.dropped, 0);
    let pairs: Vec<_> = sent.transmissions().collect();
    assert_eq!(pairs.len(), 4);
    assert!(pairs.iter().all(|(_, message)| *message == sent.message()));
    assert_eq!(pairs[2].0, FD1);
}

#[test]
fn the_source_is_never_sent_its_own_broadcast() {
    let tables = site();
    let mut out = storage();
    // From a foreign device's address (a device on this subnet can't be,
    // but the rule is by address).
    let sent = tables.original_broadcast(&context(T0), FD1, NPDU, &mut out);
    assert_eq!(
        destinations(sent.fanout.targets),
        [TWO_HOP, addr(192, 168, 2, 255, PORT), FD2]
    );
    // From a peer's address.
    let sent = tables.original_broadcast(&context(T0), TWO_HOP, NPDU, &mut out);
    assert_eq!(
        destinations(sent.fanout.targets),
        [addr(192, 168, 2, 255, PORT), FD1, FD2]
    );
}

#[test]
fn an_empty_bdt_forwards_to_foreign_devices_only() {
    let mut never_set = Small::new(OWN);
    register_all(&mut never_set, &[FD1], 60, T0);
    let mut only_own = tables(&[]);
    register_all(&mut only_own, &[FD1], 60, T0);
    for tables in [never_set, only_own] {
        let mut out = storage();
        let sent = tables.original_broadcast(&context(T0), LOCAL_DEVICE, NPDU, &mut out);
        assert_eq!(sent.fanout.targets, [target(FD1, Via::ForeignDevice)]);
    }
    let empty = Small::new(OWN);
    let mut out = storage();
    let sent = empty.original_broadcast(&context(T0), LOCAL_DEVICE, NPDU, &mut out);
    assert!(sent.fanout.targets.is_empty());
}

#[test]
fn lapsed_foreign_devices_get_nothing_even_before_the_purge() {
    let tables = site();
    let mut out = storage();
    // 60 s ttl + 30 s grace: live at 90 s, gone just after.
    let sent = tables.original_broadcast(&context(T0 + secs(90)), LOCAL_DEVICE, NPDU, &mut out);
    assert_eq!(sent.fanout.targets.len(), 4);
    let later = T0 + secs(91);
    let sent = tables.original_broadcast(&context(later), LOCAL_DEVICE, NPDU, &mut out);
    assert_eq!(
        destinations(sent.fanout.targets),
        [TWO_HOP, addr(192, 168, 2, 255, PORT)]
    );
    assert_eq!(tables.fdt.len(), 2, "a decision purges nothing");
}

#[test]
fn rows_that_share_a_destination_send_it_once() {
    let first = row(addr(192, 168, 7, 10, PORT), MASK_24);
    let second = row(addr(192, 168, 7, 20, PORT), MASK_24);
    let mut tables = tables(&[first, second]);
    // A foreign device at the same directed broadcast address, and another.
    let shadow = addr(192, 168, 7, 255, PORT);
    let other = addr(10, 0, 0, 99, PORT);
    register_all(&mut tables, &[shadow, other], 60, T0);
    let mut out = storage();
    let sent = tables.original_broadcast(&context(T0), addr(172, 16, 0, 1, PORT), NPDU, &mut out);
    assert_eq!(
        sent.fanout.targets,
        [target(shadow, Via::Peer), target(other, Via::ForeignDevice)]
    );
    assert_eq!(sent.fanout.deduplicated, 2);
}

#[test]
fn the_fanout_budget_caps_foreign_devices_but_not_peers() {
    let mut tables = tables(&[row(TWO_HOP, MASK_32)]);
    let devices: Vec<_> = (1..=6).map(|host| addr(172, 16, 0, host, PORT)).collect();
    register_all(&mut tables, &devices, 60, T0);
    let cx = Context::new(T0, 2);
    let mut out = storage();
    let sent = tables.original_broadcast(&cx, LOCAL_DEVICE, NPDU, &mut out);
    assert_eq!(
        destinations(sent.fanout.targets),
        [TWO_HOP, devices[0], devices[1]]
    );
    assert!(sent.fanout.fanout_capped);
    // The excluded source doesn't count against the budget.
    let sent = tables.original_broadcast(&cx, devices[0], NPDU, &mut out);
    assert_eq!(
        destinations(sent.fanout.targets),
        [TWO_HOP, devices[1], devices[2]]
    );
    // A budget that fits every device isn't reported as reached.
    let cx = Context::new(cx.now, 6);
    let sent = tables.original_broadcast(&cx, LOCAL_DEVICE, NPDU, &mut out);
    assert_eq!(sent.fanout.targets.len(), 7);
    assert!(!sent.fanout.fanout_capped);
    let none = Context::new(cx.now, 0);
    let sent = tables.original_broadcast(&none, LOCAL_DEVICE, NPDU, &mut out);
    assert_eq!(destinations(sent.fanout.targets), [TWO_HOP]);
    assert!(sent.fanout.fanout_capped);
}

#[test]
fn storage_too_small_drops_and_counts_the_rest() {
    let tables = site();
    let mut out = [Target::UNUSED; 3];
    let sent = tables.original_broadcast(&context(T0), LOCAL_DEVICE, NPDU, &mut out);
    assert_eq!(sent.fanout.targets.len(), 3);
    assert_eq!(sent.fanout.dropped, 1);
    let mut none: [Target; 0] = [];
    let sent = tables
        .distribute_broadcast(&context(T0), &SUBNET, FD1, NPDU, &mut none)
        .unwrap();
    assert!(sent.fanout.targets.is_empty());
    assert_eq!(sent.fanout.dropped, 4);
    assert_eq!(Small::MAX_TARGETS, 17);
}

/// Peers reach this BBMD by its own BDT row, here the appended /32 one:
/// they send to it alone, so a forward that came by unicast is broadcast on
/// the subnet, whatever mask the sender's own row has.
#[test]
fn a_forward_that_came_by_unicast_to_a_two_hop_own_row_is_rebroadcast() {
    let tables = site();
    assert_eq!(tables.bdt.row(OWN), Some(row(OWN, MASK_32)));
    let origin = addr(10, 0, 0, 77, PORT);
    let mut out = storage();
    for sender in [TWO_HOP, ONE_HOP] {
        let sent = tables
            .forwarded_npdu(
                &context(T0),
                &SUBNET,
                sender,
                Arrival::Unicast,
                origin,
                &mut out,
            )
            .unwrap();
        assert_eq!(
            sent.targets,
            [
                target(LOCAL_BROADCAST, Via::LocalBroadcast),
                target(FD1, Via::ForeignDevice),
                target(FD2, Via::ForeignDevice),
            ],
            "{sender}"
        );
        let sent = tables
            .forwarded_npdu(
                &context(T0),
                &SUBNET,
                sender,
                Arrival::Broadcast,
                origin,
                &mut out,
            )
            .unwrap();
        assert_eq!(destinations(sent.targets), [FD1, FD2], "{sender}");
    }
}

/// The case the #1623 review gave: a /32 own row and a /24 peer. The peer
/// sends to this BBMD alone, so the subnet hasn't heard the broadcast.
#[test]
fn a_32_own_row_rebroadcasts_a_24_peers_unicast_forward() {
    let mut tables = tables(&[row(OWN, MASK_32), row(ONE_HOP, MASK_24)]);
    register_all(&mut tables, &[FD1], 60, T0);
    let mut out = storage();
    let sent = tables
        .forwarded_npdu(
            &context(T0),
            &SUBNET,
            ONE_HOP,
            Arrival::Unicast,
            FD2,
            &mut out,
        )
        .unwrap();
    assert_eq!(
        sent.targets,
        [
            target(LOCAL_BROADCAST, Via::LocalBroadcast),
            target(FD1, Via::ForeignDevice)
        ]
    );
}

/// With a /24 own row peers send to this subnet's directed broadcast, so
/// every device there has the message: it goes to the foreign devices only,
/// however it arrived and whatever the sender's row says.
#[test]
fn a_one_hop_own_row_means_the_subnet_already_has_it() {
    let mut tables = tables(&[row(OWN, MASK_24), row(TWO_HOP, MASK_32)]);
    register_all(&mut tables, &[FD1, FD2], 60, T0);
    let mut out = storage();
    for arrival in [Arrival::Unicast, Arrival::Broadcast] {
        let sent = tables
            .forwarded_npdu(
                &context(T0),
                &SUBNET,
                TWO_HOP,
                arrival,
                LOCAL_DEVICE,
                &mut out,
            )
            .unwrap();
        assert_eq!(destinations(sent.targets), [FD1, FD2], "{arrival:?}");
    }
}

/// A forward sent from an address on this BBMD's own subnet came from a
/// BBMD whose subnet is this one, which heard the broadcast first hand.
#[test]
fn a_forward_from_a_sender_on_the_own_subnet_is_not_rebroadcast() {
    let neighbour = addr(192, 168, 1, 2, PORT);
    let mut tables = tables(&[row(neighbour, MASK_32), row(TWO_HOP, MASK_32)]);
    register_all(&mut tables, &[FD1], 60, T0);
    let mut known = SUBNET;
    known.mask = Some(MASK_24);
    let mut out = storage();
    let sent = tables
        .forwarded_npdu(
            &context(T0),
            &known,
            neighbour,
            Arrival::Unicast,
            LOCAL_DEVICE,
            &mut out,
        )
        .unwrap();
    assert_eq!(destinations(sent.targets), [FD1]);
    // A sender off the subnet still gets the rebroadcast.
    let sent = tables
        .forwarded_npdu(
            &context(T0),
            &known,
            TWO_HOP,
            Arrival::Unicast,
            LOCAL_DEVICE,
            &mut out,
        )
        .unwrap();
    assert_eq!(destinations(sent.targets), [LOCAL_BROADCAST, FD1]);
    // Without the mask the subnet is unknown, so the neighbour's is too.
    let sent = tables
        .forwarded_npdu(
            &context(T0),
            &SUBNET,
            neighbour,
            Arrival::Unicast,
            LOCAL_DEVICE,
            &mut out,
        )
        .unwrap();
    assert_eq!(destinations(sent.targets), [LOCAL_BROADCAST, FD1]);
}

#[test]
fn the_rebroadcast_rule_alone() {
    let two_hop_own = tables(&[row(ONE_HOP, MASK_24)]);
    let one_hop_own = tables(&[row(OWN, MASK_24), row(TWO_HOP, MASK_32)]);
    let unset = Small::new(OWN);
    // On this subnet: at this BBMD's own port, and at another, where
    // another B/IP network shares the subnet (Annex J.1.2).
    let neighbour = addr(192, 168, 1, 77, PORT);
    let other_network = addr(192, 168, 1, 77, 47809);
    for (tables, mask, sender, arrival, expected) in [
        (&two_hop_own, None, ONE_HOP, Arrival::Unicast, true),
        (&two_hop_own, None, ONE_HOP, Arrival::Broadcast, false),
        (&two_hop_own, Some(MASK_24), ONE_HOP, Arrival::Unicast, true),
        (
            &two_hop_own,
            Some(MASK_24),
            neighbour,
            Arrival::Unicast,
            false,
        ),
        (
            &two_hop_own,
            Some(MASK_24),
            other_network,
            Arrival::Unicast,
            true,
        ),
        (
            &two_hop_own,
            Some(MASK_32),
            neighbour,
            Arrival::Unicast,
            true,
        ),
        (&one_hop_own, None, TWO_HOP, Arrival::Unicast, false),
        // No own row yet: peers can only have reached it by unicast.
        (&unset, None, TWO_HOP, Arrival::Unicast, true),
    ] {
        assert_eq!(
            tables.rebroadcasts_forwarded(mask, sender, arrival),
            expected,
            "{mask:?} {sender} {arrival:?}"
        );
    }
}

#[test]
fn a_forwarded_npdu_skips_its_origin_and_the_peers() {
    let tables = site();
    let mut out = storage();
    let sent = tables
        .forwarded_npdu(
            &context(T0),
            &SUBNET,
            TWO_HOP,
            Arrival::Unicast,
            FD2,
            &mut out,
        )
        .unwrap();
    assert_eq!(
        sent.targets,
        [
            target(LOCAL_BROADCAST, Via::LocalBroadcast),
            target(FD1, Via::ForeignDevice)
        ]
    );
    let cx = Context::new(T0, 1);
    let sent = tables
        .forwarded_npdu(
            &cx,
            &SUBNET,
            TWO_HOP,
            Arrival::Broadcast,
            addr(10, 9, 9, 9, PORT),
            &mut out,
        )
        .unwrap();
    assert_eq!(destinations(sent.targets), [FD1]);
    assert!(sent.fanout_capped);
}

#[test]
fn a_forwarded_npdu_from_anyone_but_a_peer_is_refused() {
    let tables = site();
    let mut out = storage();
    for sender in [FD1, LOCAL_DEVICE, addr(10, 0, 0, 1, PORT + 1)] {
        assert_eq!(
            tables.forwarded_npdu(
                &context(T0),
                &SUBNET,
                sender,
                Arrival::Unicast,
                sender,
                &mut out
            ),
            Err(ForwardRefusal::NotFromPeer),
            "{sender}"
        );
    }
    assert_eq!(
        tables.forwarded_npdu(
            &context(T0),
            &SUBNET,
            OWN,
            Arrival::Broadcast,
            TWO_HOP,
            &mut out
        ),
        Err(ForwardRefusal::OwnEcho)
    );
}

#[test]
fn a_registered_foreign_device_distributes_to_the_subnet_peers_and_other_devices() {
    let tables = site();
    let mut out = storage();
    let sent = tables
        .distribute_broadcast(&context(T0), &SUBNET, FD1, NPDU, &mut out)
        .unwrap();
    assert_eq!(sent.origin, FD1);
    assert_eq!(
        sent.fanout.targets,
        [
            target(LOCAL_BROADCAST, Via::LocalBroadcast),
            target(TWO_HOP, Via::Peer),
            target(addr(192, 168, 2, 255, PORT), Via::Peer),
            target(FD2, Via::ForeignDevice),
        ]
    );
}

#[test]
fn the_local_broadcast_is_not_deduplicated_against_peers() {
    // A peer row whose directed broadcast is this subnet's broadcast address.
    let same_subnet = row(addr(192, 168, 1, 2, PORT), MASK_24);
    let mut tables = tables(&[same_subnet]);
    register_all(&mut tables, &[FD1], 60, T0);
    let mut out = storage();
    let sent = tables
        .distribute_broadcast(&context(T0), &SUBNET, FD1, NPDU, &mut out)
        .unwrap();
    assert_eq!(
        sent.fanout.targets,
        [
            target(LOCAL_BROADCAST, Via::LocalBroadcast),
            target(LOCAL_BROADCAST, Via::Peer)
        ]
    );
    assert_eq!(sent.fanout.deduplicated, 0);
}

#[test]
fn only_a_live_registration_may_distribute() {
    let tables = site();
    let mut out = storage();
    let nak = Err(DistributeRefusal::NotRegistered);
    assert_eq!(
        tables.distribute_broadcast(&context(T0), &SUBNET, LOCAL_DEVICE, NPDU, &mut out),
        nak
    );
    assert_eq!(
        tables.distribute_broadcast(&context(T0), &SUBNET, TWO_HOP, NPDU, &mut out),
        nak
    );
    // Same IP, another port: not the registrant.
    assert_eq!(
        tables.distribute_broadcast(
            &context(T0),
            &SUBNET,
            addr(172, 16, 0, 5, PORT + 1),
            NPDU,
            &mut out
        ),
        nak
    );
    assert!(tables
        .distribute_broadcast(&context(T0 + secs(90)), &SUBNET, FD1, NPDU, &mut out)
        .is_ok());
    assert_eq!(
        tables.distribute_broadcast(&context(T0 + secs(91)), &SUBNET, FD1, NPDU, &mut out),
        nak
    );
}

#[test]
fn the_bbmds_own_broadcast_comes_from_its_own_address() {
    let mut tables = site();
    // A foreign device registered from the BBMD's own address is skipped.
    register_all(&mut tables, &[OWN], 60, T0);
    let mut out = storage();
    let sent = tables.own_broadcast(&context(T0), NPDU, &mut out);
    assert_eq!(sent.origin, OWN);
    assert_eq!(
        destinations(sent.fanout.targets),
        [TWO_HOP, addr(192, 168, 2, 255, PORT), FD1, FD2]
    );
}

#[test]
fn target_lists_alone() {
    let tables = site();
    let mut out = storage();
    let all = tables.broadcast_targets(&context(T0), TWO_HOP, &mut out);
    assert_eq!(
        destinations(all.targets),
        [addr(192, 168, 2, 255, PORT), FD1, FD2]
    );
    let fds = tables.foreign_device_targets(&context(T0), FD2, &mut out);
    assert_eq!(fds.targets, [target(FD1, Via::ForeignDevice)]);
    assert_eq!(
        Target::UNUSED.destination,
        SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)
    );
}

#[test]
fn larger_tables_work_the_same() {
    let mut tables: Tables<128, 128> = Tables::new(OWN);
    let rows: Vec<_> = (1..=100)
        .map(|host| row(addr(10, 1, host, 1, PORT), MASK_24))
        .collect();
    tables
        .bdt
        .set(bacnet_bip_codec::BdtEntries::from_entries(&rows))
        .unwrap();
    for host in 1..=100 {
        let outcome = tables.fdt.register(
            addr(172, 16, 1, host, PORT),
            60,
            T0,
            &support::OPEN,
            bacnet_bip_core::bbmd::Reservation::Ordinary,
            || true,
        );
        assert!(outcome.result.is_ok());
    }
    let mut out = [Target::UNUSED; Tables::<128, 128>::MAX_TARGETS];
    let cx = Context::new(T0, 128);
    let sent = tables.original_broadcast(&cx, LOCAL_DEVICE, NPDU, &mut out);
    assert_eq!(sent.fanout.targets.len(), 200);
    assert_eq!(
        sent.fanout.targets[0].destination,
        addr(10, 1, 1, 255, PORT)
    );
}

#[test]
fn refusals_carry_the_nak_annex_j_gives() {
    for refusal in [ForwardRefusal::NotFromPeer, ForwardRefusal::OwnEcho] {
        assert_eq!(refusal.result_code(), None, "{refusal:?}");
        assert!(!refusal.to_string().is_empty());
    }
    let refusal = DistributeRefusal::NotRegistered;
    assert_eq!(
        refusal.result_code(),
        bacnet_bip_codec::BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK
    );
    assert!(!refusal.to_string().is_empty());
}
