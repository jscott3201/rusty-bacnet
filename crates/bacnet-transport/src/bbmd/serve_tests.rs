//! The hosted BBMD's management answers through the core's `serve` (#1773).

use std::time::{Duration, Instant};

use super::*;

const ADMIN: [u8; 4] = [10, 0, 0, 1];
const DEVICE: [u8; 4] = [10, 0, 0, 5];
const PORT: u16 = 0xBAC0;

fn bbmd() -> BbmdState {
    let mut state = BbmdState::new([192, 168, 1, 1], PORT);
    state.enable_foreign_device_registration(ForeignDevicePolicy::default());
    state.set_management_acl(vec![ADMIN]);
    state
}

fn delete(state: &mut BbmdState, now: Instant) -> Reply {
    let request = Request::DeleteFdtEntry {
        sender: address(ADMIN, PORT),
        entry: address(DEVICE, PORT),
    };
    state.serve(request, now)
}

/// Past its time-to-live and the 30 s grace a registration no longer
/// exists (Annex J.5.2.3), so deleting it gets the NAK (J.5.2.1.2), where
/// the BBMD once answered success for an entry it hadn't purged yet. The
/// purge that removes it is counted.
#[test]
fn delete_of_a_lapsed_registration_naks_and_counts_the_purge() {
    let mut state = bbmd();
    let now = Instant::now();
    state.insert_fdt_entry_for_test(FdtEntry {
        ip: DEVICE,
        port: PORT,
        ttl: 60,
        registered_at: now - Duration::from_secs(91),
    });
    assert_eq!(state.fdt_len_for_test(), 1, "lapsed but not purged yet");

    assert_eq!(
        delete(&mut state, now),
        Reply::Result(BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK)
    );
    assert_eq!(state.fdt_len_for_test(), 0);
    assert_eq!(state.fdt_counters().registrations_expired, 1);
}

/// At exactly time-to-live plus grace the registration is still live, and
/// the delete succeeds.
#[test]
fn delete_at_the_last_live_instant_succeeds() {
    let mut state = bbmd();
    let now = Instant::now();
    state.insert_fdt_entry_for_test(FdtEntry {
        ip: DEVICE,
        port: PORT,
        ttl: 60,
        registered_at: now - Duration::from_secs(90),
    });
    assert_eq!(
        delete(&mut state, now),
        Reply::Result(BvlcResultCode::SUCCESSFUL_COMPLETION)
    );
    assert_eq!(state.fdt_len_for_test(), 0);
    assert_eq!(state.fdt_counters().registrations_expired, 0);
}

/// The Delete-FDT-Entry access list fails closed: a sender it doesn't name
/// is refused and the registration stays.
#[test]
fn delete_from_a_sender_off_the_access_list_naks() {
    let mut state = bbmd();
    let now = Instant::now();
    assert_eq!(
        state.register_foreign_device_at(DEVICE, PORT, 60, now),
        BvlcResultCode::SUCCESSFUL_COMPLETION
    );
    let request = Request::DeleteFdtEntry {
        sender: address([10, 0, 0, 2], PORT),
        entry: address(DEVICE, PORT),
    };
    assert_eq!(
        state.serve(request, now),
        Reply::Result(BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK)
    );
    assert_eq!(state.fdt_len_for_test(), 1);
}

/// A refused registration doesn't use up the rate limit: only an accepted
/// one is recorded against it.
#[test]
fn only_an_accepted_registration_counts_against_the_rate_limit() {
    let mut state = bbmd();
    // Every slot reserved, and one registration per source per window.
    state.enable_foreign_device_registration(ForeignDevicePolicy {
        reserved_capacity: BbmdState::MAX_FDT_ENTRIES,
        registration_rate_per_source: 1,
        rate_window: Duration::from_secs(60),
        ..Default::default()
    });
    let now = Instant::now();
    assert_eq!(
        state.register_foreign_device_at(DEVICE, PORT, 60, now),
        BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK
    );
    let counters = state.fdt_counters();
    assert_eq!(counters.capacity_exhausted, 1);
    assert_eq!(counters.registrations_rejected, 1);

    // Same window: the refusal left the source's one registration unused.
    state.enable_foreign_device_registration(ForeignDevicePolicy {
        registration_rate_per_source: 1,
        rate_window: Duration::from_secs(60),
        ..Default::default()
    });
    assert_eq!(
        state.register_foreign_device_at(DEVICE, PORT, 60, now),
        BvlcResultCode::SUCCESSFUL_COMPLETION
    );
    // Now it is used.
    assert_eq!(
        state.register_foreign_device_at(DEVICE, PORT + 1, 60, now),
        BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK
    );
    let counters = state.fdt_counters();
    assert_eq!(counters.registrations_accepted, 1);
    assert_eq!(counters.registrations_rejected, 2);
}

/// Read-FDT lists the live registrations with the seconds they have left,
/// purging and counting the lapsed ones.
#[test]
fn read_fdt_lists_live_rows_and_purges_lapsed_ones() {
    let mut state = bbmd();
    let now = Instant::now();
    state.insert_fdt_entry_for_test(FdtEntry {
        ip: [10, 0, 0, 9],
        port: PORT,
        ttl: 10,
        registered_at: now - Duration::from_secs(41),
    });
    assert_eq!(
        state.register_foreign_device_at(DEVICE, PORT, 60, now),
        BvlcResultCode::SUCCESSFUL_COMPLETION
    );
    let reply = state.serve(
        Request::ReadFdt {
            sender: address(ADMIN, PORT),
        },
        now + Duration::from_secs(5),
    );
    let device = address(DEVICE, PORT);
    assert_eq!(
        reply,
        Reply::FdtAck(vec![codec::FdtEntry::new(device, 60, 85)])
    );
    assert_eq!(state.fdt_counters().registrations_expired, 1);
}

/// Read-BDT answers with every row, this BBMD's own included.
#[test]
fn read_bdt_lists_every_row() {
    let mut state = bbmd();
    let peer = BdtEntry {
        ip: [10, 1, 0, 1],
        port: PORT,
        broadcast_mask: [255, 255, 255, 255],
    };
    state.set_bdt(vec![peer]).unwrap();
    let reply = state.serve(
        Request::ReadBdt {
            sender: address(ADMIN, PORT),
        },
        Instant::now(),
    );
    assert_eq!(reply, Reply::BdtAck(state.bdt_wire_entries()));
    assert_eq!(state.bdt_wire_entries().len(), 2);
}

/// Without a policy every registration is refused, and the core's purge
/// before it is counted.
#[test]
fn a_refused_no_policy_registration_counts_the_purge_before_it() {
    let mut state = BbmdState::new([192, 168, 1, 1], PORT);
    let now = Instant::now();
    state.insert_fdt_entry_for_test(FdtEntry {
        ip: DEVICE,
        port: PORT,
        ttl: 10,
        registered_at: now - Duration::from_secs(41),
    });
    assert_eq!(
        state.register_foreign_device_at([10, 0, 0, 6], PORT, 60, now),
        BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK
    );
    let counters = state.fdt_counters();
    assert_eq!(counters.registrations_rejected, 1);
    assert_eq!(counters.registrations_expired, 1);
    assert_eq!(state.fdt_len_for_test(), 0);
}
