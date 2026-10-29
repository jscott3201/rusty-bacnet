//! The foreign device table: registration, renewal, limits, expiry with the
//! grace period, and what a Read-Foreign-Device-Table-Ack reports (Annex J.5.2).

mod support;

use bacnet_bip_codec::{BvlcResultCode, FdtEntry};
use bacnet_bip_core::bbmd::{
    Fdt, GRACE_PERIOD, Registered, Registration, RegistrationLimits, RegistrationRefusal,
    Reservation, StorageFull,
};
use bacnet_bip_core::{Duration, Instant};
use support::{OPEN, PORT, addr, secs};

const T0: Instant = Instant::from_secs(1_000);

fn register<const N: usize>(
    fdt: &mut Fdt<N>,
    device: core::net::SocketAddrV4,
    ttl: u16,
    now: Instant,
) -> Result<Registered, RegistrationRefusal> {
    fdt.register(device, ttl, now, &OPEN, Reservation::Ordinary, || true)
        .result
}

#[test]
fn a_registration_is_stored_then_renewed_in_place() {
    let mut fdt: Fdt<4> = Fdt::new();
    assert!(fdt.is_empty());
    assert_eq!(Fdt::<4>::CAPACITY, 4);
    let a = addr(10, 0, 0, 5, PORT);
    let b = addr(10, 0, 0, 6, PORT);
    assert_eq!(register(&mut fdt, a, 60, T0), Ok(Registered::New));
    assert_eq!(register(&mut fdt, b, 60, T0), Ok(Registered::New));
    let later = T0 + secs(10);
    assert_eq!(register(&mut fdt, a, 120, later), Ok(Registered::Renewed));
    assert_eq!(fdt.len(), 2);
    // Renewal keeps the table order.
    assert_eq!(
        fdt.registrations(),
        [
            Registration::new(a, 120, later),
            Registration::new(b, 60, T0),
        ]
    );
    assert_eq!(fdt.get(b).map(|entry| entry.ttl), Some(60));
    assert_eq!(fdt.get(addr(10, 0, 0, 7, PORT)), None);
}

#[test]
fn time_to_live_outside_the_limits_is_refused_without_touching_the_entry() {
    let mut fdt: Fdt<4> = Fdt::new();
    let mut limits = RegistrationLimits::default();
    limits.min_ttl = 30;
    limits.max_ttl = 300;
    let device = addr(10, 0, 0, 1, PORT);
    assert!(
        fdt.register(device, 60, T0, &limits, Reservation::Ordinary, || true)
            .result
            .is_ok()
    );
    for ttl in [0, 10, 29, 301, u16::MAX] {
        let outcome = fdt.register(
            device,
            ttl,
            T0 + secs(1),
            &limits,
            Reservation::Ordinary,
            || panic!("not asked"),
        );
        assert_eq!(
            outcome.result,
            Err(RegistrationRefusal::TimeToLive),
            "{ttl}"
        );
    }
    assert_eq!(fdt.registrations()[0].ttl, 60);
    assert_eq!(fdt.registrations()[0].registered_at, T0);
    // Zero is refused even when the limits would allow it.
    let mut zero = limits;
    zero.min_ttl = 0;
    assert_eq!(
        fdt.register(device, 0, T0, &zero, Reservation::Ordinary, || true)
            .result,
        Err(RegistrationRefusal::TimeToLive)
    );
    for ttl in [30, 300] {
        assert_eq!(
            fdt.register(device, ttl, T0, &limits, Reservation::Ordinary, || true)
                .result,
            Ok(Registered::Renewed)
        );
    }
}

#[test]
fn the_admission_check_runs_after_the_time_to_live_and_can_refuse() {
    let mut fdt: Fdt<4> = Fdt::new();
    let device = addr(10, 0, 0, 1, PORT);
    let mut asked = 0;
    let outcome = fdt.register(device, 60, T0, &OPEN, Reservation::Ordinary, || {
        asked += 1;
        false
    });
    assert_eq!(outcome.result, Err(RegistrationRefusal::NotAdmitted));
    assert_eq!(asked, 1);
    assert!(fdt.is_empty());
    // A renewal is asked too.
    register(&mut fdt, device, 60, T0).unwrap();
    let outcome = fdt.register(
        device,
        90,
        T0 + secs(1),
        &OPEN,
        Reservation::Ordinary,
        || false,
    );
    assert_eq!(outcome.result, Err(RegistrationRefusal::NotAdmitted));
    assert_eq!(fdt.registrations()[0].ttl, 60);
}

/// The purge comes first, so a refused registration still frees the
/// lapsed slots and reports them.
#[test]
fn a_refused_registration_still_purges() {
    for refusal in [
        RegistrationRefusal::TimeToLive,
        RegistrationRefusal::NotAdmitted,
    ] {
        let mut fdt: Fdt<4> = Fdt::new();
        register(&mut fdt, addr(10, 0, 0, 1, PORT), 1, T0).unwrap();
        register(&mut fdt, addr(10, 0, 0, 2, PORT), 1, T0).unwrap();
        register(&mut fdt, addr(10, 0, 0, 3, PORT), 60, T0).unwrap();
        let (ttl, admit) = match refusal {
            RegistrationRefusal::TimeToLive => (0, true),
            _ => (60, false),
        };
        let outcome = fdt.register(
            addr(10, 0, 0, 9, PORT),
            ttl,
            T0 + secs(32),
            &OPEN,
            Reservation::Ordinary,
            || admit,
        );
        assert_eq!(outcome.result, Err(refusal));
        assert_eq!(outcome.purged, 2, "{refusal:?}");
        assert_eq!(fdt.len(), 1);
    }
}

#[test]
fn one_address_may_hold_its_quota_across_ports() {
    let mut fdt: Fdt<8> = Fdt::new();
    let mut limits = OPEN;
    limits.max_entries_per_source = 2;
    let ip = |port| addr(10, 0, 0, 1, port);
    assert!(
        fdt.register(ip(1), 60, T0, &limits, Reservation::Ordinary, || true)
            .result
            .is_ok()
    );
    assert!(
        fdt.register(ip(2), 60, T0, &limits, Reservation::Ordinary, || true)
            .result
            .is_ok()
    );
    assert_eq!(
        fdt.register(ip(3), 60, T0, &limits, Reservation::Ordinary, || true)
            .result,
        Err(RegistrationRefusal::SourceQuota)
    );
    // Renewing a held port costs nothing, and another address is unaffected.
    assert_eq!(
        fdt.register(ip(1), 60, T0, &limits, Reservation::Ordinary, || true)
            .result,
        Ok(Registered::Renewed)
    );
    assert!(
        fdt.register(
            addr(10, 0, 0, 2, 1),
            60,
            T0,
            &limits,
            Reservation::Ordinary,
            || true
        )
        .result
        .is_ok()
    );
}

#[test]
fn a_full_table_refuses_new_devices_but_renews_old_ones() {
    let mut fdt: Fdt<3> = Fdt::new();
    for host in 1..=3 {
        register(&mut fdt, addr(10, 0, 0, host, PORT), 60, T0).unwrap();
    }
    assert_eq!(
        register(&mut fdt, addr(10, 0, 0, 4, PORT), 60, T0),
        Err(RegistrationRefusal::TableFull)
    );
    assert_eq!(
        register(&mut fdt, addr(10, 0, 0, 2, PORT), 90, T0),
        Ok(Registered::Renewed)
    );
    // Once one lapses the slot is free again: register purges first.
    let outcome = fdt.register(
        addr(10, 0, 0, 4, PORT),
        60,
        T0 + secs(91),
        &OPEN,
        Reservation::Ordinary,
        || true,
    );
    assert_eq!(outcome.purged, 2);
    assert_eq!(outcome.result, Ok(Registered::New));
    assert_eq!(fdt.len(), 2);
}

#[test]
fn reserved_capacity_is_kept_for_reserved_sources() {
    let mut fdt: Fdt<4> = Fdt::new();
    let mut limits = OPEN;
    limits.reserved_capacity = 1;
    for host in 1..=3 {
        let outcome = fdt.register(
            addr(10, 0, 0, host, PORT),
            60,
            T0,
            &limits,
            Reservation::Ordinary,
            || true,
        );
        assert!(outcome.result.is_ok());
    }
    let ordinary = fdt.register(
        addr(10, 0, 0, 4, PORT),
        60,
        T0,
        &limits,
        Reservation::Ordinary,
        || true,
    );
    assert_eq!(ordinary.result, Err(RegistrationRefusal::TableFull));
    let reserved = fdt.register(
        addr(10, 0, 0, 9, PORT),
        60,
        T0,
        &limits,
        Reservation::Reserved,
        || true,
    );
    assert_eq!(reserved.result, Ok(Registered::New));
    let over = fdt.register(
        addr(10, 0, 0, 10, PORT),
        60,
        T0,
        &limits,
        Reservation::Reserved,
        || true,
    );
    assert_eq!(over.result, Err(RegistrationRefusal::TableFull));
    // Reserving more than the table holds leaves ordinary sources nothing.
    let mut small: Fdt<2> = Fdt::new();
    let mut greedy = OPEN;
    greedy.reserved_capacity = 5;
    let outcome = small.register(
        addr(10, 0, 0, 1, PORT),
        60,
        T0,
        &greedy,
        Reservation::Ordinary,
        || true,
    );
    assert_eq!(outcome.result, Err(RegistrationRefusal::TableFull));
}

#[test]
fn a_registration_lapses_just_after_its_ttl_plus_the_grace_period() {
    let entry = Registration::new(addr(10, 0, 0, 5, PORT), 60, T0);
    assert_eq!(GRACE_PERIOD, secs(30));
    assert_eq!(entry.lifetime(), secs(90));
    let deadline = T0 + secs(90);
    assert!(!entry.is_expired_at(T0));
    assert!(
        !entry.is_expired_at(T0 + secs(70)),
        "inside the grace period"
    );
    assert!(!entry.is_expired_at(deadline), "at exactly ttl + grace");
    assert!(entry.is_expired_at(deadline + Duration::from_nanos(1)));
    // A clock that reads before the registration counts no time passed.
    assert!(!entry.is_expired_at(Instant::ZERO));

    let mut fdt: Fdt<2> = Fdt::new();
    fdt.restore(entry).unwrap();
    assert!(fdt.is_registered_at(entry.address, deadline));
    assert_eq!(fdt.purge(deadline), 0);
    assert!(!fdt.is_registered_at(entry.address, deadline + Duration::from_nanos(1)));
    assert_eq!(fdt.len(), 1, "lapsed but not yet purged");
    assert_eq!(fdt.live_at(deadline + Duration::from_nanos(1)).count(), 0);
    assert_eq!(fdt.purge(deadline + Duration::from_nanos(1)), 1);
    assert!(fdt.is_empty());
}

#[test]
fn seconds_remaining_count_down_from_ttl_plus_grace() {
    let entry = Registration::new(addr(10, 0, 0, 5, PORT), 60, T0);
    let half = Duration::from_millis(500);
    for (elapsed, left) in [
        (Duration::ZERO, 90),
        (half, 90),
        (secs(1), 89),
        (secs(10) + half, 80),
        (secs(89), 1),
        (secs(89) + half, 1),
        (secs(90), 0),
        (secs(90) + half, 0),
        (secs(1_000_000), 0),
    ] {
        assert_eq!(
            entry.seconds_remaining_at(T0 + elapsed),
            left,
            "{elapsed:?}"
        );
    }
    assert_eq!(entry.seconds_remaining_at(Instant::ZERO), 90);
    // Seconds past what a u32 counts still leave nothing.
    assert_eq!(entry.seconds_remaining_at(Instant::MAX), 0);
    assert_eq!(
        entry.wire_entry_at(T0 + secs(30)),
        FdtEntry::new(entry.address, 60, 60)
    );
}

#[test]
fn seconds_remaining_cap_at_the_field_maximum() {
    let entry = Registration::new(addr(10, 0, 0, 5, PORT), u16::MAX, T0);
    assert_eq!(entry.seconds_remaining_at(T0), u16::MAX);
    assert_eq!(entry.seconds_remaining_at(T0 + secs(30)), u16::MAX);
    assert_eq!(entry.seconds_remaining_at(T0 + secs(31)), u16::MAX - 1);
    assert_eq!(entry.seconds_remaining_at(T0 + secs(65_565)), 0);
}

#[test]
fn purge_and_delete_keep_the_order_of_the_rest() {
    let mut fdt: Fdt<4> = Fdt::new();
    let a = addr(10, 0, 0, 1, PORT);
    let b = addr(10, 0, 0, 2, PORT);
    let c = addr(10, 0, 0, 3, PORT);
    register(&mut fdt, a, 60, T0).unwrap();
    register(&mut fdt, b, 10, T0).unwrap();
    register(&mut fdt, c, 60, T0).unwrap();
    assert_eq!(fdt.purge(T0 + secs(41)), 1);
    let left: Vec<_> = fdt
        .registrations()
        .iter()
        .map(|entry| entry.address)
        .collect();
    assert_eq!(left, [a, c]);
    assert!(fdt.delete(a));
    assert!(!fdt.delete(a));
    assert!(!fdt.delete(b));
    assert_eq!(fdt.registrations()[0].address, c);
}

#[test]
fn a_lapsed_registration_can_be_deleted_before_it_is_purged() {
    let mut fdt: Fdt<2> = Fdt::new();
    let device = addr(10, 0, 0, 1, PORT);
    register(&mut fdt, device, 1, T0).unwrap();
    assert!(!fdt.is_registered_at(device, T0 + secs(32)));
    assert!(fdt.delete(device));
}

#[test]
fn restore_replaces_or_appends_without_checks() {
    let mut fdt: Fdt<2> = Fdt::new();
    let a = Registration::new(addr(10, 0, 0, 1, PORT), 0, T0);
    let b = Registration::new(addr(10, 0, 0, 2, PORT), 5, T0);
    fdt.restore(a).unwrap();
    fdt.restore(b).unwrap();
    let moved = Registration::new(a.address, a.ttl, T0 + secs(3));
    fdt.restore(moved).unwrap();
    assert_eq!(fdt.registrations(), [moved, b]);
    let c = Registration::new(addr(10, 0, 0, 3, PORT), b.ttl, b.registered_at);
    let full = StorageFull {
        needed: 3,
        available: 2,
    };
    assert_eq!(fdt.restore(c), Err(full));
    assert!(!full.to_string().is_empty());
}

#[test]
fn wire_entries_report_the_live_registrations() {
    let mut fdt: Fdt<4> = Fdt::default();
    let a = addr(10, 0, 0, 1, PORT);
    let b = addr(10, 0, 0, 2, 47809);
    register(&mut fdt, a, 60, T0).unwrap();
    register(&mut fdt, b, 1, T0).unwrap();
    let now = T0 + secs(20);
    let mut out = [FdtEntry::new(addr(0, 0, 0, 0, 0), 0, 0); 4];
    let table = fdt.wire_entries(now, &mut out).unwrap();
    let rows: Vec<FdtEntry> = table.iter().collect();
    assert_eq!(rows, [FdtEntry::new(a, 60, 70), FdtEntry::new(b, 1, 11)]);
    // b has lapsed 40 s on: it is left out though still stored.
    let mut out = [FdtEntry::new(addr(0, 0, 0, 0, 0), 0, 0); 1];
    let table = fdt.wire_entries(T0 + secs(40), &mut out).unwrap();
    assert_eq!(table.len(), 1);
    let mut none: [FdtEntry; 0] = [];
    let full = StorageFull {
        needed: 2,
        available: 0,
    };
    assert_eq!(fdt.wire_entries(now, &mut none), Err(full));
    // Room for one of the two: it says how many there are.
    let mut one = [FdtEntry::new(addr(0, 0, 0, 0, 0), 0, 0); 1];
    let full = StorageFull {
        needed: 2,
        available: 1,
    };
    assert_eq!(fdt.wire_entries(now, &mut one), Err(full));
}

/// What storage too small for the rows needed counts only the live
/// registrations, not the lapsed ones still stored on either side of where
/// it ran out.
#[test]
fn storage_too_small_counts_only_live_registrations() {
    let mut fdt: Fdt<4> = Fdt::new();
    // Live, lapsed, live, lapsed, in table order, 40 s on.
    for (host, ttl) in [(1, 60), (2, 1), (3, 60), (4, 1)] {
        register(&mut fdt, addr(10, 0, 0, host, PORT), ttl, T0).unwrap();
    }
    let now = T0 + secs(40);
    assert_eq!(fdt.len(), 4);
    assert_eq!(fdt.live_at(now).count(), 2);
    let mut one = [FdtEntry::new(addr(0, 0, 0, 0, 0), 0, 0); 1];
    let full = StorageFull {
        needed: 2,
        available: 1,
    };
    assert_eq!(fdt.wire_entries(now, &mut one), Err(full));
}

#[test]
fn refusals_say_why() {
    for refusal in [
        RegistrationRefusal::TimeToLive,
        RegistrationRefusal::NotAdmitted,
        RegistrationRefusal::SourceQuota,
        RegistrationRefusal::TableFull,
    ] {
        assert!(!refusal.to_string().is_empty());
        assert_eq!(
            refusal.result_code(),
            BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK
        );
    }
}

#[test]
fn debug_shows_only_the_registrations_in_use() {
    let mut fdt: Fdt<64> = Fdt::new();
    register(&mut fdt, addr(10, 0, 0, 5, PORT), 60, T0).unwrap();
    let text = format!("{fdt:?}");
    assert!(text.contains("10.0.0.5:47808"), "{text}");
    assert!(!text.contains("0.0.0.0"), "{text}");
}
