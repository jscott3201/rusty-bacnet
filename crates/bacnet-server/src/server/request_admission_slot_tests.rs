//! Slot accounting of [`Admission`] (#1713): the caps, the counters and the
//! release of a slot, pinned at the admission layer itself.
use super::*;
use bacnet_types::MacAddr;

fn peer(mac: u8) -> CanonicalRequester {
    CanonicalRequester::Direct(MacAddr::from_slice(&[mac]))
}

fn policy(global: usize, per_peer: usize) -> RequestAdmissionPolicy {
    RequestAdmissionPolicy {
        max_confirmed_in_flight: global + 1,
        max_confirmed_in_flight_per_peer: per_peer,
        confirmed_recovery_reserve: 1,
        max_recovery_in_flight_per_peer: 1,
        ..RequestAdmissionPolicy::default()
    }
}

#[test]
fn global_cap_rejects_at_the_limit_and_a_dropped_guard_frees_its_slot() {
    let admission = Admission::new(policy(2, 2)).unwrap();
    let first = admission
        .try_enter(Class::Confirmed, peer(1), false)
        .unwrap();
    let _second = admission
        .try_enter(Class::Confirmed, peer(2), false)
        .unwrap();
    assert_eq!(
        admission.try_enter(Class::Confirmed, peer(3), false).err(),
        Some(Rejection::Overloaded)
    );
    let counters = admission.snapshot();
    assert_eq!(counters.confirmed_active, 2);
    assert_eq!(counters.confirmed_admitted_total, 2);
    assert_eq!(counters.confirmed_overloaded_total, 1);
    assert_eq!(counters.confirmed_global_overloaded_total, 1);
    assert_eq!(counters.confirmed_peer_overloaded_total, 0);
    drop(first);
    assert_eq!(admission.snapshot().confirmed_active, 1);
    let _third = admission
        .try_enter(Class::Confirmed, peer(3), false)
        .unwrap();
    assert_eq!(admission.snapshot().confirmed_admitted_total, 3);
}

#[test]
fn a_peer_rejection_gives_back_the_global_slot_it_took() {
    let admission = Admission::new(policy(3, 1)).unwrap();
    let _a = admission
        .try_enter(Class::Confirmed, peer(1), false)
        .unwrap();
    for _ in 0..5 {
        assert_eq!(
            admission.try_enter(Class::Confirmed, peer(1), false).err(),
            Some(Rejection::Overloaded)
        );
    }
    // Had any of the five kept its slot, the global cap of three would be
    // reached before both of these.
    let _b = admission
        .try_enter(Class::Confirmed, peer(2), false)
        .unwrap();
    let _c = admission
        .try_enter(Class::Confirmed, peer(3), false)
        .unwrap();
    assert_eq!(
        admission.try_enter(Class::Confirmed, peer(4), false).err(),
        Some(Rejection::Overloaded)
    );
    let counters = admission.snapshot();
    assert_eq!(counters.confirmed_peer_overloaded_total, 5);
    assert_eq!(counters.confirmed_global_overloaded_total, 1);
    assert_eq!(counters.confirmed_active, 3);
}

#[test]
fn recovery_slots_are_their_own_and_closed_takes_none() {
    let admission = Admission::new(policy(1, 1)).unwrap();
    let ordinary = admission
        .try_enter(Class::Confirmed, peer(1), false)
        .unwrap();
    // The reserve admits a recovery handler while ordinary capacity is full.
    let recovery = admission
        .try_enter(Class::Recovery, peer(2), false)
        .unwrap();
    assert_eq!(
        admission.try_enter(Class::Recovery, peer(3), false).err(),
        Some(Rejection::Overloaded)
    );
    let counters = admission.snapshot();
    assert_eq!(counters.recovery_active, 1);
    assert_eq!(counters.confirmed_active, 2);
    assert_eq!(counters.recovery_overloaded_total, 1);
    drop(recovery);
    drop(ordinary);
    assert_eq!(
        admission.try_enter(Class::Confirmed, peer(1), true).err(),
        Some(Rejection::Closed)
    );
    assert_eq!(admission.snapshot().confirmed_shutdown_rejected_total, 1);
    // A closed rejection took no slot.
    let _again = admission
        .try_enter(Class::Confirmed, peer(1), false)
        .unwrap();
    let _recovery = admission
        .try_enter(Class::Recovery, peer(2), false)
        .unwrap();
}

#[test]
fn abort_workers_cap_at_eight_without_peer_registration() {
    let admission = Admission::new(RequestAdmissionPolicy::default()).unwrap();
    let workers: Vec<_> = (0..8)
        .map(|_| admission.try_enter(Class::Abort, peer(1), false).unwrap())
        .collect();
    assert_eq!(
        admission.try_enter(Class::Abort, peer(2), false).err(),
        Some(Rejection::Overloaded)
    );
    assert_eq!(admission.peer_entries()[2], 0);
    assert_eq!(admission.snapshot().confirmed_fallback_dropped_total, 1);
    drop(workers);
    assert_eq!(admission.snapshot().abort_active, 0);
    let _next = admission.try_enter(Class::Abort, peer(2), false).unwrap();
}
