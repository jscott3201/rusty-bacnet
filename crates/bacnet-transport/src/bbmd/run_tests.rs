//! Whole runs of the hosted BBMD, with results worked out by hand: a full
//! FDT, expiry exactly at the time-to-live plus the grace period, and the
//! default fan-out budget. They replace the fixed runs of the differential
//! test that compared the BBMD with the code it replaced (#1623).

use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::{Duration, Instant};

use bacnet_bip_codec::BvlcResultCode;
use bacnet_bip_core::bbmd::{DistributeRefusal, ForwardRefusal};
use bacnet_bip_core::Arrival;

use super::*;

const OWN: ([u8; 4], u16) = ([10, 0, 0, 1], 0xBAC0);
const LOCAL_BROADCAST: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 255), 0xBAC0);
const SUCCESS: BvlcResultCode = BvlcResultCode::SUCCESSFUL_COMPLETION;
const NAK: BvlcResultCode = BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK;

fn at(ip: [u8; 4], port: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::from(ip), port)
}

fn bbmd(policy: ForeignDevicePolicy) -> BbmdState {
    let mut bbmd = BbmdState::new(OWN.0, OWN.1);
    bbmd.enable_foreign_device_registration(policy);
    bbmd
}

/// 128 slots, 2 kept for one reserved source: ordinary sources fill 126,
/// the reserved one the last 2, and then everything lapses together.
#[test]
fn a_full_fdt_keeps_its_reserved_slots_then_lapses_whole() {
    let reserved = [10, 7, 0, 1];
    let mut bbmd = bbmd(ForeignDevicePolicy {
        reserved_capacity: 2,
        reserved_sources: vec![reserved],
        max_entries_per_source: 200,
        registration_rate_per_source: 1_000,
        registration_rate_global: 1_000,
        max_fdt_fanout: 200,
        ..ForeignDevicePolicy::default()
    });
    let ordinary: Vec<[u8; 4]> = (0..130u16)
        .map(|i| [10, 6, (i / 100) as u8, (i % 100 + 1) as u8])
        .collect();
    let mut now = Instant::now();
    let mut results = Vec::new();
    for ip in &ordinary {
        now += Duration::from_millis(5);
        results.push(bbmd.register_foreign_device_at(*ip, 0xBAC0, 60, now));
    }
    assert!(results[..126].iter().all(|result| *result == SUCCESS));
    assert!(results[126..].iter().all(|result| *result == NAK));
    let ports: Vec<_> = (0..4)
        .map(|offset| bbmd.register_foreign_device_at(reserved, 0xBAC0 + offset, 60, now))
        .collect();
    assert_eq!(ports, [SUCCESS, SUCCESS, NAK, NAK]);
    assert_eq!(
        bbmd.fdt_counters(),
        FdtCounters {
            registrations_accepted: 128,
            registrations_rejected: 6,
            registrations_expired: 0,
            capacity_exhausted: 6,
            fanout_budget_reached: 0,
            destinations_deduplicated: 0,
        }
    );

    // Its own broadcast reaches every registration, in the order they came.
    let forward = bbmd.forward_own_broadcast(&[0x01, 0x00], now);
    let expected: Vec<SocketAddrV4> = ordinary[..126]
        .iter()
        .map(|ip| at(*ip, 0xBAC0))
        .chain([at(reserved, 0xBAC0), at(reserved, 0xBAC1)])
        .collect();
    assert_eq!(forward.origin, at(OWN.0, OWN.1));
    assert_eq!(forward.local_broadcast, None);
    assert_eq!(forward.remote, expected);

    // 60 s and the 30 s grace after the last registration, and a second
    // more: every one has lapsed.
    assert_eq!(bbmd.purge_expired_at(now + Duration::from_secs(91)), 128);
    assert_eq!(bbmd.fdt_counters().registrations_expired, 128);
}

/// A registration still counts at exactly its time-to-live plus 30 s, and
/// not a nanosecond later.
#[test]
fn a_registration_lapses_just_after_its_ttl_plus_the_grace_period() {
    let mut bbmd = bbmd(ForeignDevicePolicy::default());
    let device = [10, 0, 1, 2];
    let brief = [10, 0, 1, 3];
    let t0 = Instant::now();
    assert_eq!(
        bbmd.register_foreign_device_at(device, 0xBAC0, 60, t0),
        SUCCESS
    );
    assert_eq!(
        bbmd.register_foreign_device_at(brief, 0xBAC0, 1, t0),
        SUCCESS
    );
    let nanosecond = Duration::from_nanos(1);

    let now = t0 + Duration::from_secs(31);
    let forward = bbmd
        .forward_distribute_broadcast(at(brief, 0xBAC0), &[1], LOCAL_BROADCAST, now)
        .unwrap();
    assert_eq!(forward.local_broadcast, Some(LOCAL_BROADCAST));
    assert_eq!(forward.remote, [at(device, 0xBAC0)]);
    let refused = bbmd.forward_distribute_broadcast(
        at(brief, 0xBAC0),
        &[1],
        LOCAL_BROADCAST,
        now + nanosecond,
    );
    assert_eq!(refused.unwrap_err(), DistributeRefusal::NotRegistered);
    assert_eq!(bbmd.fdt_counters().registrations_expired, 1);

    let now = t0 + Duration::from_secs(90);
    let forward = bbmd.forward_original_broadcast(at(OWN.0, 0xBAC1), &[1], now);
    assert_eq!(forward.remote, [at(device, 0xBAC0)]);
    assert_eq!(bbmd.purge_expired_at(now), 0);
    let forward = bbmd.forward_original_broadcast(at(OWN.0, 0xBAC1), &[1], now + nanosecond);
    assert!(forward.remote.is_empty());
    assert_eq!(bbmd.fdt_counters().registrations_expired, 2);
    let refused = bbmd.forward_distribute_broadcast(
        at(device, 0xBAC0),
        &[1],
        LOCAL_BROADCAST,
        now + nanosecond,
    );
    assert_eq!(refused.unwrap_err(), DistributeRefusal::NotRegistered);
}

/// Without a policy a message goes to at most 32 foreign devices: the
/// registrations a removed policy admitted are kept and forwarded to.
#[test]
fn without_a_policy_a_message_reaches_32_foreign_devices() {
    let mut bbmd = bbmd(ForeignDevicePolicy {
        max_entries_per_source: 200,
        registration_rate_per_source: 1_000,
        registration_rate_global: 1_000,
        max_fdt_fanout: 100,
        ..ForeignDevicePolicy::default()
    });
    let now = Instant::now();
    let devices: Vec<[u8; 4]> = (1..=40).map(|host| [10, 8, 0, host]).collect();
    for ip in &devices {
        assert_eq!(
            bbmd.register_foreign_device_at(*ip, 0xBAC0, 60, now),
            SUCCESS
        );
    }
    bbmd.set_foreign_device_policy(None);

    let forward = bbmd.forward_original_broadcast(at(OWN.0, 0xBAC1), &[1], now);
    let first_32: Vec<_> = devices[..32].iter().map(|ip| at(*ip, 0xBAC0)).collect();
    assert_eq!(forward.remote, first_32);
    assert_eq!(bbmd.fdt_counters().fanout_budget_reached, 1);

    // Excluding the first, the next 32.
    let targets = bbmd.fdt_forwarding_targets_at(devices[0], 0xBAC0, now);
    let next_32: Vec<_> = devices[1..33].iter().map(|ip| (*ip, 0xBAC0)).collect();
    assert_eq!(targets, next_32);
    assert_eq!(bbmd.fdt_counters().fanout_budget_reached, 2);
}

/// Each registration purges the lapsed ones first, and counts them, even
/// when it is refused.
#[test]
fn a_registration_counts_the_registrations_it_purges() {
    let mut bbmd = bbmd(ForeignDevicePolicy::default());
    let t0 = Instant::now();
    assert_eq!(
        bbmd.register_foreign_device_at([10, 0, 1, 2], 0xBAC0, 1, t0),
        SUCCESS
    );
    assert_eq!(
        bbmd.register_foreign_device_at([10, 0, 1, 3], 0xBAC0, 2, t0),
        SUCCESS
    );
    // A zero time-to-live is refused, after the first registration lapsed.
    let refused =
        bbmd.register_foreign_device_at([10, 0, 1, 4], 0xBAC0, 0, t0 + Duration::from_secs(32));
    assert_eq!(refused, NAK);
    assert_eq!(bbmd.fdt_counters().registrations_expired, 1);
    let accepted =
        bbmd.register_foreign_device_at([10, 0, 1, 4], 0xBAC0, 60, t0 + Duration::from_secs(33));
    assert_eq!(accepted, SUCCESS);
    assert_eq!(bbmd.fdt_counters().registrations_expired, 2);
    assert_eq!(bbmd.fdt_len_for_test(), 1);
}

/// A Forwarded-NPDU purges lapsed registrations only when it is accepted,
/// as the receive path always has: a refused one, from no peer, leaves the
/// table and its counters alone.
#[test]
fn only_an_accepted_forward_purges() {
    let peer = ([10, 9, 0, 1], 0xBAC0);
    let mut bbmd = bbmd(ForeignDevicePolicy::default());
    bbmd.set_bdt(vec![BdtEntry {
        ip: peer.0,
        port: peer.1,
        broadcast_mask: [255; 4],
    }])
    .unwrap();
    let t0 = Instant::now();
    assert_eq!(
        bbmd.register_foreign_device_at([10, 0, 1, 2], 0xBAC0, 1, t0),
        SUCCESS
    );
    let later = t0 + Duration::from_secs(32);
    let origin = at([10, 9, 0, 7], 0xBAC0);

    let refused = bbmd.forward_forwarded_npdu(
        at([10, 9, 0, 2], 0xBAC0),
        Arrival::Unicast,
        origin,
        LOCAL_BROADCAST,
        later,
    );
    assert_eq!(refused, Err(ForwardRefusal::NotFromPeer));
    assert_eq!(bbmd.fdt_len_for_test(), 1);
    assert_eq!(bbmd.fdt_counters().registrations_expired, 0);

    let forward = bbmd
        .forward_forwarded_npdu(
            at(peer.0, peer.1),
            Arrival::Unicast,
            origin,
            LOCAL_BROADCAST,
            later,
        )
        .unwrap();
    assert_eq!(forward.local_broadcast, Some(LOCAL_BROADCAST));
    assert!(forward.remote.is_empty(), "the lapsed device gets nothing");
    assert_eq!(bbmd.fdt_len_for_test(), 0);
    assert_eq!(bbmd.fdt_counters().registrations_expired, 1);
}

/// The receive path applies the subnet mask the host reported: a peer on
/// this BBMD's subnet at its own port reached the subnet's devices itself,
/// one at another port serves another B/IP network there (Annex J.1.2).
#[test]
fn the_receive_path_skips_the_rebroadcast_for_a_peer_on_the_own_subnet_and_port() {
    let neighbour = ([10, 0, 0, 2], OWN.1);
    let other_network = ([10, 0, 0, 3], OWN.1 + 1);
    let remote = ([10, 9, 0, 1], OWN.1);
    let mut bbmd = bbmd(ForeignDevicePolicy::default());
    let rows = [neighbour, other_network, remote].map(|(ip, port)| BdtEntry {
        ip,
        port,
        broadcast_mask: [255; 4],
    });
    bbmd.set_bdt(rows.to_vec()).unwrap();
    let now = Instant::now();
    let origin = at([10, 9, 0, 7], 0xBAC0);
    let local = |bbmd: &mut BbmdState, (ip, port): ([u8; 4], u16)| {
        bbmd.forward_forwarded_npdu(at(ip, port), Arrival::Unicast, origin, LOCAL_BROADCAST, now)
            .unwrap()
            .local_broadcast
    };

    // Without a mask every peer's unicast forward is rebroadcast.
    for peer in [neighbour, other_network, remote] {
        assert_eq!(local(&mut bbmd, peer), Some(LOCAL_BROADCAST), "{peer:?}");
    }
    bbmd.set_subnet_mask(Some(Ipv4Addr::new(255, 255, 255, 0)));
    assert_eq!(local(&mut bbmd, neighbour), None);
    assert_eq!(local(&mut bbmd, other_network), Some(LOCAL_BROADCAST));
    assert_eq!(local(&mut bbmd, remote), Some(LOCAL_BROADCAST));
}
