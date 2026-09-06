use std::time::{Duration, Instant};

use super::*;
use bacnet_types::enums::BvlcResultCode;

fn make_unconfigured_bbmd() -> BbmdState {
    BbmdState::new([192, 168, 1, 1], 0xBAC0)
}

#[test]
fn bbmd_without_policy_naks_registration_fail_closed() {
    let mut bbmd = make_unconfigured_bbmd();
    assert!(bbmd.foreign_device_policy().is_none());

    let result = bbmd.register_foreign_device([10, 0, 0, 5], 0xBAC0, 60);
    assert_eq!(result, BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK);
    assert!(bbmd.fdt().is_empty());

    let counters = bbmd.fdt_counters();
    assert_eq!(counters.registrations_rejected, 1);
    assert_eq!(counters.registrations_accepted, 0);
}

#[test]
fn allowed_sources_acl_enforces_membership_and_empty_denies_all() {
    let mut bbmd = make_unconfigured_bbmd();
    // Some([ip]) allows only listed IP
    bbmd.enable_foreign_device_registration(ForeignDevicePolicy {
        allowed_sources: Some(vec![[10, 0, 0, 1]]),
        ..Default::default()
    });

    // Allowed source succeeds
    let res1 = bbmd.register_foreign_device([10, 0, 0, 1], 0xBAC0, 60);
    assert_eq!(res1, BvlcResultCode::SUCCESSFUL_COMPLETION);

    // Unlisted source fails
    let res2 = bbmd.register_foreign_device([10, 0, 0, 2], 0xBAC0, 60);
    assert_eq!(res2, BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK);

    // Empty list denies all (fail-closed)
    bbmd.enable_foreign_device_registration(ForeignDevicePolicy {
        allowed_sources: Some(vec![]),
        ..Default::default()
    });
    let res3 = bbmd.register_foreign_device([10, 0, 0, 1], 0xBAC0, 60);
    assert_eq!(res3, BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK);

    // None allows any source
    bbmd.enable_foreign_device_registration(ForeignDevicePolicy {
        allowed_sources: None,
        ..Default::default()
    });
    let res4 = bbmd.register_foreign_device([10, 0, 0, 2], 0xBAC0, 60);
    assert_eq!(res4, BvlcResultCode::SUCCESSFUL_COMPLETION);
}

#[test]
fn per_source_quota_does_not_deny_other_sources() {
    let mut bbmd = make_unconfigured_bbmd();
    bbmd.enable_foreign_device_registration(ForeignDevicePolicy {
        max_entries_per_source: 3,
        registration_rate_per_source: 10,
        ..Default::default()
    });

    let src_a = [10, 0, 0, 1];
    let src_b = [10, 0, 0, 2];

    // Source A registers 3 distinct ports (filling quota)
    assert_eq!(
        bbmd.register_foreign_device(src_a, 0xBAC0, 60),
        BvlcResultCode::SUCCESSFUL_COMPLETION
    );
    assert_eq!(
        bbmd.register_foreign_device(src_a, 0xBAC1, 60),
        BvlcResultCode::SUCCESSFUL_COMPLETION
    );
    assert_eq!(
        bbmd.register_foreign_device(src_a, 0xBAC2, 60),
        BvlcResultCode::SUCCESSFUL_COMPLETION
    );

    // 4th port from Source A must be rejected
    assert_eq!(
        bbmd.register_foreign_device(src_a, 0xBAC3, 60),
        BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK
    );

    // Re-registering existing (ip, port) does NOT consume new quota and succeeds
    assert_eq!(
        bbmd.register_foreign_device(src_a, 0xBAC0, 120),
        BvlcResultCode::SUCCESSFUL_COMPLETION
    );

    // Source B can register up to its own quota unaffected by Source A
    assert_eq!(
        bbmd.register_foreign_device(src_b, 0xBAC0, 60),
        BvlcResultCode::SUCCESSFUL_COMPLETION
    );
    assert_eq!(
        bbmd.register_foreign_device(src_b, 0xBAC1, 60),
        BvlcResultCode::SUCCESSFUL_COMPLETION
    );
    assert_eq!(bbmd.fdt().len(), 5);

    let counters = bbmd.fdt_counters();
    assert_eq!(counters.registrations_accepted, 6);
    assert_eq!(counters.registrations_rejected, 1);
}

#[test]
fn overlong_ttl_and_over_rate_rejected_without_mutating_existing_entries() {
    let mut bbmd = make_unconfigured_bbmd();
    bbmd.enable_foreign_device_registration(ForeignDevicePolicy {
        min_ttl: 30,
        max_ttl: 300,
        registration_rate_per_source: 3,
        registration_rate_global: 10,
        rate_window: Duration::from_secs(10),
        ..Default::default()
    });

    let t0 = Instant::now();
    let ip = [10, 0, 0, 1];
    let port = 0xBAC0;

    // 1st registration: valid TTL 60 at t0
    assert_eq!(
        bbmd.register_foreign_device_at(ip, port, 60, t0),
        BvlcResultCode::SUCCESSFUL_COMPLETION
    );
    assert_eq!(bbmd.fdt().len(), 1);
    assert_eq!(bbmd.fdt()[0].ttl, 60);
    assert_eq!(bbmd.fdt()[0].registered_at, t0);

    let t1 = t0 + Duration::from_secs(1);

    // Overlong TTL (500 > max_ttl 300) must NAK and preserve existing entry
    assert_eq!(
        bbmd.register_foreign_device_at(ip, port, 500, t1),
        BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK
    );
    assert_eq!(bbmd.fdt()[0].ttl, 60);
    assert_eq!(bbmd.fdt()[0].registered_at, t0);

    // Below min_ttl (10 < min_ttl 30) must NAK and preserve existing entry
    assert_eq!(
        bbmd.register_foreign_device_at(ip, port, 10, t1),
        BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK
    );
    assert_eq!(bbmd.fdt()[0].ttl, 60);
    assert_eq!(bbmd.fdt()[0].registered_at, t0);

    // Zero TTL must NAK and preserve existing entry
    assert_eq!(
        bbmd.register_foreign_device_at(ip, port, 0, t1),
        BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK
    );
    assert_eq!(bbmd.fdt()[0].ttl, 60);
    assert_eq!(bbmd.fdt()[0].registered_at, t0);

    // 2nd valid registration within window at t1
    assert_eq!(
        bbmd.register_foreign_device_at(ip, port, 90, t1),
        BvlcResultCode::SUCCESSFUL_COMPLETION
    );
    assert_eq!(bbmd.fdt()[0].ttl, 90);
    assert_eq!(bbmd.fdt()[0].registered_at, t1);

    // 3rd valid registration within window at t1 + 1s (hits per-source limit of 3)
    let t2 = t1 + Duration::from_secs(1);
    assert_eq!(
        bbmd.register_foreign_device_at(ip, port, 120, t2),
        BvlcResultCode::SUCCESSFUL_COMPLETION
    );
    assert_eq!(bbmd.fdt()[0].ttl, 120);
    assert_eq!(bbmd.fdt()[0].registered_at, t2);

    // 4th registration within same rate window exceeds rate limit -> NAK and preserve entry!
    let t3 = t2 + Duration::from_secs(1);
    assert_eq!(
        bbmd.register_foreign_device_at(ip, port, 150, t3),
        BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK
    );
    assert_eq!(bbmd.fdt()[0].ttl, 120);
    assert_eq!(bbmd.fdt()[0].registered_at, t2);

    // After rate window expires (t0 + 12s), registration is admitted again
    let t4 = t0 + Duration::from_secs(12);
    assert_eq!(
        bbmd.register_foreign_device_at(ip, port, 180, t4),
        BvlcResultCode::SUCCESSFUL_COMPLETION
    );
    assert_eq!(bbmd.fdt()[0].ttl, 180);
    assert_eq!(bbmd.fdt()[0].registered_at, t4);
}

#[test]
fn capacity_pressure_preserves_reserved_capacity() {
    let mut bbmd = make_unconfigured_bbmd();
    let reserved_ip = [10, 0, 0, 99];
    bbmd.enable_foreign_device_registration(ForeignDevicePolicy {
        reserved_capacity: 10,
        reserved_sources: vec![reserved_ip],
        max_entries_per_source: 128,
        registration_rate_per_source: 200,
        registration_rate_global: 200,
        ..Default::default()
    });

    let unreserved_limit = BbmdState::MAX_FDT_ENTRIES - 10; // 118

    // Fill unreserved capacity (118 entries) with unique IPs
    for i in 0..unreserved_limit {
        let ip = [10, 1, (i / 256) as u8, (i % 256) as u8];
        assert_eq!(
            bbmd.register_foreign_device(ip, 0xBAC0, 60),
            BvlcResultCode::SUCCESSFUL_COMPLETION
        );
    }
    assert_eq!(bbmd.fdt().len(), unreserved_limit);

    // Attempting 119th entry from unreserved source must NAK due to capacity pressure
    let unreserved_ip = [10, 2, 0, 1];
    assert_eq!(
        bbmd.register_foreign_device(unreserved_ip, 0xBAC0, 60),
        BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK
    );
    assert_eq!(bbmd.fdt_counters().capacity_exhausted, 1);

    // Reserved source is admitted into reserved slots
    for i in 0..10 {
        assert_eq!(
            bbmd.register_foreign_device(reserved_ip, 0xBAC0 + (i as u16), 60),
            BvlcResultCode::SUCCESSFUL_COMPLETION
        );
    }
    assert_eq!(bbmd.fdt().len(), BbmdState::MAX_FDT_ENTRIES);

    // Total table is now at MAX_FDT_ENTRIES (128). Even reserved source cannot exceed 128.
    assert_eq!(
        bbmd.register_foreign_device(reserved_ip, 0xFFF0, 60),
        BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK
    );
    assert_eq!(bbmd.fdt_counters().capacity_exhausted, 2);
}

#[test]
fn forwarding_targets_fanout_budget_capped_and_counted() {
    let mut bbmd = make_unconfigured_bbmd();
    bbmd.enable_foreign_device_registration(ForeignDevicePolicy {
        max_fdt_fanout: 5,
        max_entries_per_source: 128,
        registration_rate_per_source: 200,
        registration_rate_global: 200,
        ..Default::default()
    });

    // Populate 12 foreign devices
    for i in 0..12 {
        let ip = [10, 0, 0, i as u8 + 1];
        assert_eq!(
            bbmd.register_foreign_device(ip, 0xBAC0, 60),
            BvlcResultCode::SUCCESSFUL_COMPLETION
        );
    }

    assert_eq!(bbmd.fdt_counters().fanout_budget_reached, 0);

    // Forwarding broadcast from an external subnet device
    let targets = bbmd.forwarding_targets([192, 168, 1, 100], 0xBAC0);
    // BDT has self only, so 0 BDT targets; FDT targets capped to max_fdt_fanout (5)
    assert_eq!(targets.len(), 5);
    assert_eq!(bbmd.fdt_counters().fanout_budget_reached, 1);

    // Second broadcast forwarding again increments counter
    let targets2 = bbmd.forwarding_targets([192, 168, 1, 100], 0xBAC0);
    assert_eq!(targets2.len(), 5);
    assert_eq!(bbmd.fdt_counters().fanout_budget_reached, 2);
}

#[test]
fn counters_accurate_for_all_events() {
    let mut bbmd = make_unconfigured_bbmd();
    bbmd.enable_foreign_device_registration(ForeignDevicePolicy {
        min_ttl: 10,
        max_ttl: 100,
        max_entries_per_source: 2,
        reserved_capacity: 1,
        reserved_sources: vec![[10, 0, 0, 99]],
        max_fdt_fanout: 2,
        registration_rate_per_source: 5,
        registration_rate_global: 10,
        ..Default::default()
    });

    let t0 = Instant::now();

    // 1. Accepted registration
    assert_eq!(
        bbmd.register_foreign_device_at([10, 0, 0, 1], 0xBAC0, 30, t0),
        BvlcResultCode::SUCCESSFUL_COMPLETION
    );
    assert_eq!(bbmd.fdt_counters().registrations_accepted, 1);
    assert_eq!(bbmd.fdt_counters().registrations_rejected, 0);

    // 2. Rejected: TTL too short (< 10)
    assert_eq!(
        bbmd.register_foreign_device_at([10, 0, 0, 2], 0xBAC0, 5, t0),
        BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK
    );
    assert_eq!(bbmd.fdt_counters().registrations_rejected, 1);

    // 3. Rejected: Quota exceeded
    assert_eq!(
        bbmd.register_foreign_device_at([10, 0, 0, 1], 0xBAC1, 30, t0),
        BvlcResultCode::SUCCESSFUL_COMPLETION
    );
    assert_eq!(
        bbmd.register_foreign_device_at([10, 0, 0, 1], 0xBAC2, 30, t0),
        BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK
    );
    assert_eq!(bbmd.fdt_counters().registrations_rejected, 2);

    // 4. Fanout budget reached
    assert_eq!(
        bbmd.register_foreign_device_at([10, 0, 0, 3], 0xBAC0, 30, t0),
        BvlcResultCode::SUCCESSFUL_COMPLETION
    );
    // Now we have 3 entries in FDT, max_fdt_fanout is 2
    let targets = bbmd.forwarding_targets_at([192, 168, 1, 100], 0xBAC0, t0);
    assert_eq!(targets.len(), 2);
    assert_eq!(bbmd.fdt_counters().fanout_budget_reached, 1);

    // 5. Expired registration purged
    // Entries have TTL 30 + grace 30 = 60s
    let t_expired = t0 + Duration::from_secs(65);
    let purged = bbmd.purge_expired_at(t_expired);
    assert_eq!(purged, 3);
    assert_eq!(bbmd.fdt_counters().registrations_expired, 3);
    assert!(bbmd.fdt().is_empty());
}

#[test]
fn fdt_forwarding_targets_fanout_budget_and_exclusion() {
    let mut bbmd = make_unconfigured_bbmd();
    bbmd.enable_foreign_device_registration(ForeignDevicePolicy {
        max_fdt_fanout: 3,
        max_entries_per_source: 10,
        registration_rate_per_source: 100,
        registration_rate_global: 100,
        ..Default::default()
    });

    for i in 1..=5 {
        assert_eq!(
            bbmd.register_foreign_device([10, 0, 0, i], 0xBAC0, 60),
            BvlcResultCode::SUCCESSFUL_COMPLETION
        );
    }

    assert_eq!(bbmd.fdt_counters().fanout_budget_reached, 0);

    // If originator is one of the foreign devices, exclude it and cap to 3
    let targets = bbmd.fdt_forwarding_targets([10, 0, 0, 1], 0xBAC0);
    assert_eq!(targets.len(), 3);
    assert!(!targets.contains(&([10, 0, 0, 1], 0xBAC0)));
    assert_eq!(bbmd.fdt_counters().fanout_budget_reached, 1);

    // If originator is external, return 3 and increment counter again
    let targets2 = bbmd.fdt_forwarding_targets([192, 168, 1, 1], 0xBAC0);
    assert_eq!(targets2.len(), 3);
    assert_eq!(bbmd.fdt_counters().fanout_budget_reached, 2);
}
