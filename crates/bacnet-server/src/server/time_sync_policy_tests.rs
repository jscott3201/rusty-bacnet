use super::*;
use crate::server::test_transport::TestTransport;
use crate::server::{BACnetServer, ServerConfig};
use bacnet_objects::database::ObjectDatabase;
use bacnet_transport::port::TransportProvenance;
use bacnet_types::{
    primitives::{Date, Time},
    MacAddr,
};
use bytes::Bytes;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

fn received(mac: &[u8], routed: Option<(u16, &[u8])>) -> ReceivedApdu {
    ReceivedApdu {
        direct_response: None,
        apdu: Bytes::new(),
        source_mac: MacAddr::from_slice(mac),
        ingress_network: None,
        source_network: routed.map(|(network, address)| NpduAddress {
            network,
            mac_address: MacAddr::from_slice(address),
        }),
        link_layer_group: false,
        is_group: false,
        global_broadcast: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

fn rate(burst_capacity: u32) -> TimeSyncRateLimit {
    TimeSyncRateLimit {
        max_per_second: 1.0,
        burst_capacity,
    }
}

#[test]
fn exact_sources_and_configured_empty_do_not_fall_back_to_router() {
    let restriction = TimeSyncSourceRestriction::new(vec![
        TimeSyncSource::Direct(vec![1, 2, 3, 4, 5, 6]), // SC VMAC-shaped claimed source
        TimeSyncSource::Routed {
            network: 7,
            address: vec![8, 9],
        },
    ])
    .unwrap();
    for (mac, route, expected) in [
        (&[1, 2, 3, 4, 5, 6][..], None, true),
        (&[1, 2, 3, 4, 5][..], None, false),
        (&[1, 2, 3, 4, 5, 7][..], None, false),
        (&[99][..], Some((7, &[8, 9][..])), true),
        (&[98][..], Some((7, &[8, 9][..])), true),
        (&[99][..], Some((8, &[8, 9][..])), false),
        (&[99][..], Some((7, &[8][..])), false),
        (&[1, 2, 3, 4, 5, 6][..], Some((0, &[][..])), false),
        (&[1, 2, 3, 4, 5, 6][..], Some((7, &[10][..])), false),
    ] {
        let context = received(mac, route);
        assert_eq!(
            restriction.allows(mac, context.source_network.as_ref(), None),
            expected
        );
        assert!(!TimeSyncSourceRestriction::new(vec![]).unwrap().allows(
            mac,
            context.source_network.as_ref(),
            None
        ));
    }
    assert_eq!(
        format!("{restriction:?}"),
        "TimeSyncSourceRestriction { entries: 2, .. }"
    );
}

#[test]
fn rejects_invalid_allowlists_at_construction() {
    for source in [
        TimeSyncSource::Direct(vec![]),
        TimeSyncSource::Direct(vec![1; 256]),
        TimeSyncSource::Routed {
            network: 0,
            address: vec![1],
        },
        TimeSyncSource::Routed {
            network: 65535,
            address: vec![1],
        },
        TimeSyncSource::Routed {
            network: 1,
            address: vec![],
        },
        TimeSyncSource::Routed {
            network: 1,
            address: vec![1; 256],
        },
    ] {
        assert!(TimeSyncSourceRestriction::new(vec![source]).is_err());
    }
    let source = TimeSyncSource::Routed {
        network: 65534,
        address: vec![1; BACnetAddress::MAX_MAC_LEN],
    };
    assert!(TimeSyncSourceRestriction::new(vec![source.clone(); 256]).is_ok());
    assert!(TimeSyncSourceRestriction::new(vec![source; 257]).is_err());
}

/// An entry holds to [`BACnetAddress::MAX_MAC_LEN`], the longest source the
/// network layer delivers (#1266): 18 octets are accepted and matched in either
/// form, and 19 are refused with the error shape the DCC and COV policies use.
/// A received source past the bound gets no rate-limit key either.
#[test]
fn time_sync_source_entries_hold_to_the_bacnet_address_bound() {
    let longest = BACnetAddress::MAX_MAC_LEN;
    let entries = |length: usize| {
        [
            TimeSyncSource::Direct(vec![7; length]),
            TimeSyncSource::Routed {
                network: 65534,
                address: vec![7; length],
            },
        ]
    };
    let restriction = TimeSyncSourceRestriction::new(entries(longest).to_vec()).unwrap();
    let route = NpduAddress {
        network: 65534,
        mac_address: MacAddr::from_slice(&[7; 18]),
    };
    assert!(restriction.allows(&[7; 18], None, None));
    assert!(restriction.allows(&[1], Some(&route), None));
    for source in entries(longest + 1) {
        assert!(
            matches!(TimeSyncSourceRestriction::new(vec![source.clone()]),
                Err(Error::Encoding(m)) if m == "time sync: source address must contain 1..=18 octets"),
            "{source:?}"
        );
    }
    for (mac, route) in [
        (&[7; 18][..], None),
        (&[1][..], Some((65534, &[7; 18][..]))),
    ] {
        assert!(TimeSyncSource::from_received(&received(mac, route), None).is_ok());
    }
    for (mac, route) in [
        (&[7; 19][..], None),
        (&[1][..], Some((65534, &[7; 19][..]))),
    ] {
        assert!(TimeSyncSource::from_received(&received(mac, route), None).is_err());
    }
}

#[test]
fn step_cap_exact_boundary_both_directions_and_utc_local_parity() {
    let frame = ClockFrame {
        local_date: Date {
            year: 124,
            month: 7,
            day: 4,
            day_of_week: 4,
        },
        local_time: Time {
            hour: 22,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
        utc_offset: 300,
        daylight_savings_status: true,
    };
    let local =
        super::super::clock::date_time_to_hundredths(frame.local_date, frame.local_time).unwrap();
    for is_utc in [false, true] {
        // UTC is the next civil day; offset is west of UTC with active DST.
        let current = local + if is_utc { 4 * 360_000 } else { 0 };
        for sign in [-1, 1] {
            let delta = step_hundredths(current + sign * 100, is_utc, frame);
            assert_eq!(delta, Some(100));
            assert!(check_step(delta, Some(Duration::from_secs(1))).is_ok());
            assert!(check_step(delta, Some(Duration::from_nanos(999_999_999))).is_err());
            assert!(check_step(
                step_hundredths(current + sign * 101, is_utc, frame),
                Some(Duration::from_secs(1))
            )
            .is_err());
        }
        assert!(check_step(
            step_hundredths(current, is_utc, frame),
            Some(Duration::ZERO)
        )
        .is_ok());
    }
    assert!(check_step(None, None).is_ok());
    assert!(check_step(None, Some(Duration::MAX)).is_err());
    assert!(check_step(Some(1), Some(Duration::ZERO)).is_err());
}

#[test]
fn time_sync_rates_bound_bursts_and_restore_cadence_without_denial_debits() {
    let limiter = TimeSyncLimiter::new(TimeSyncPolicy {
        per_source_rate: Some(rate(2)),
        global_rate: Some(rate(3)),
        ..Default::default()
    });
    let a = received(&[1], None);
    let b = received(&[2], None);
    let now = runtime_clock::now();
    for _ in 0..2 {
        limiter.apply_at(&a, None, now, || Ok(())).unwrap();
    }
    assert!(limiter
        .apply_at(&a, None, now, || panic!("source limited"))
        .is_err());
    limiter.apply_at(&b, None, now, || Ok(())).unwrap();
    assert!(limiter
        .apply_at(&b, None, now, || panic!("global limited"))
        .is_err());
    assert!(limiter
        .apply_at(&a, None, now + Duration::from_millis(999), || panic!(
            "early refill"
        ))
        .is_err());
    for seconds in 1..=5 {
        let now = now + Duration::from_secs(seconds);
        limiter.apply_at(&a, None, now, || Ok(())).unwrap();
        assert!(limiter
            .apply_at(&b, None, now, || panic!("global limited"))
            .is_err());
    }
}

#[test]
fn coalescing_and_source_capacity_preserve_active_routed_identity() {
    let limiter = TimeSyncLimiter::new(TimeSyncPolicy {
        per_source_rate: Some(rate(1)),
        coalesce_window: Duration::from_secs(2),
        max_sources: 1,
        ..Default::default()
    });
    let a = received(&[1], Some((7, &[8])));
    let same_via_other_router = received(&[2], Some((7, &[8])));
    let b = received(&[1], Some((8, &[8])));
    let now = runtime_clock::now();
    limiter.apply_at(&a, None, now, || Ok(())).unwrap();
    assert!(limiter
        .apply_at(
            &same_via_other_router,
            None,
            now + Duration::from_secs(1),
            || panic!("coalesced")
        )
        .is_err());
    assert!(limiter
        .apply_at(&b, None, now + Duration::from_secs(1), || panic!(
            "table full"
        ))
        .is_err());
    // Denials did not extend the original coalescing window.
    limiter
        .apply_at(&b, None, now + Duration::from_secs(2), || Ok(()))
        .unwrap();
    assert_eq!(limiter.state.lock().unwrap().sources.len(), 1);
    assert!(limiter
        .apply_at(&a, None, now + Duration::from_secs(2), || panic!(
            "cannot evict active"
        ))
        .is_err());
}

#[test]
fn failed_step_does_not_spend_rate_or_coalescing_budget() {
    let limiter = TimeSyncLimiter::new(TimeSyncPolicy {
        per_source_rate: Some(rate(1)),
        global_rate: Some(rate(1)),
        coalesce_window: Duration::from_secs(10),
        ..Default::default()
    });
    let a = received(&[1], None);
    let now = runtime_clock::now();
    assert!(limiter
        .apply_at(&a, None, now, || Err(denied("step cap exceeded")))
        .is_err());
    assert!(limiter.state.lock().unwrap().sources.is_empty());
    limiter.apply_at(&a, None, now, || Ok(())).unwrap();
    assert!(limiter
        .apply_at(&a, None, now, || panic!("limited"))
        .is_err());
}

#[test]
fn limits_are_independent_and_monotonic_time_does_not_refill_backwards() {
    for policy in [
        TimeSyncPolicy {
            per_source_rate: Some(rate(1)),
            ..Default::default()
        },
        TimeSyncPolicy {
            global_rate: Some(rate(1)),
            ..Default::default()
        },
        TimeSyncPolicy {
            coalesce_window: Duration::from_secs(1),
            ..Default::default()
        },
    ] {
        let limiter = TimeSyncLimiter::new(policy.clone());
        let now = runtime_clock::now();
        let a = received(&[1], None);
        let b = received(&[2], None);
        limiter.apply_at(&a, None, now, || Ok(())).unwrap();
        assert!(limiter
            .apply_at(&a, None, now - Duration::from_secs(1), || panic!(
                "backwards"
            ))
            .is_err());
        assert_eq!(
            limiter.apply_at(&b, None, now, || Ok(())).is_ok(),
            policy.global_rate.is_none()
        );
        limiter
            .apply_at(&a, None, now + Duration::from_secs(1), || Ok(()))
            .unwrap();
    }
}

#[test]
fn concurrent_time_sync_admission_is_bounded_and_check_apply_is_serialized() {
    let limiter = Arc::new(TimeSyncLimiter::new(TimeSyncPolicy {
        global_rate: Some(rate(4)),
        ..Default::default()
    }));
    let applied = Arc::new(AtomicUsize::new(0));
    let now = runtime_clock::now();
    std::thread::scope(|scope| {
        for _ in 0..32 {
            scope.spawn(|| {
                let _ = limiter.apply_at(&received(&[1], None), None, now, || {
                    let before = applied.load(Ordering::SeqCst);
                    std::thread::yield_now();
                    applied.store(before + 1, Ordering::SeqCst);
                    Ok(())
                });
            });
        }
    });
    assert_eq!(applied.load(Ordering::SeqCst), 4);
}

#[tokio::test(start_paused = true)]
async fn time_sync_defaults_and_all_builders_validate_before_start_or_dial() {
    let default = TimeSyncPolicy::default();
    assert!(default.enabled);
    assert!(default.source_restriction.is_none());
    assert!(default.max_step.is_none());
    assert!(default.per_source_rate.is_none());
    assert!(default.global_rate.is_none());
    assert!(default.coalesce_window.is_zero());
    assert!(default.global_coalesce_window.is_zero());
    assert_eq!(ServerConfig::default().time_sync_policy, default);
    default.validate().unwrap();
    let mut invalid = vec![
        TimeSyncPolicy {
            max_sources: 0,
            ..Default::default()
        },
        TimeSyncPolicy {
            max_sources: 65537,
            ..Default::default()
        },
    ];
    for limit in [
        TimeSyncRateLimit {
            max_per_second: 0.0,
            burst_capacity: 1,
        },
        TimeSyncRateLimit {
            max_per_second: -1.0,
            burst_capacity: 1,
        },
        TimeSyncRateLimit {
            max_per_second: f64::NAN,
            burst_capacity: 1,
        },
        TimeSyncRateLimit {
            max_per_second: f64::INFINITY,
            burst_capacity: 1,
        },
        rate(0),
    ] {
        invalid.push(TimeSyncPolicy {
            per_source_rate: Some(limit),
            ..Default::default()
        });
        invalid.push(TimeSyncPolicy {
            enabled: false,
            global_rate: Some(limit),
            ..Default::default()
        });
    }
    for policy in invalid {
        let config = ServerConfig {
            time_sync_policy: policy.clone(),
            ..Default::default()
        };
        let direct = BACnetServer::start(
            config.clone(),
            ObjectDatabase::new(),
            TestTransport::never_start(),
        )
        .await
        .err();
        let clockless = BACnetServer::start_clockless(
            config,
            ObjectDatabase::new(),
            TestTransport::never_start(),
        )
        .await
        .err();
        let generic = BACnetServer::generic_builder()
            .transport(TestTransport::never_start())
            .time_sync_policy(policy.clone())
            .build()
            .await
            .err();
        let bip = BACnetServer::bip_builder()
            .port(0)
            .time_sync_policy(policy.clone())
            .build()
            .await
            .err();
        for error in [direct, clockless, generic, bip] {
            assert!(
                matches!(error, Some(Error::Encoding(message)) if message.starts_with("time sync:"))
            );
        }
        #[cfg(feature = "sc-tls")]
        {
            let error = BACnetServer::sc_builder()
                .hub_url("not-a-websocket-url")
                .tls_config(crate::server::sc_builder::test_tls_config())
                .device_uuid(crate::server::sc_builder::TEST_DEVICE_UUID)
                .time_sync_policy(policy)
                .build()
                .await
                .err();
            assert!(
                matches!(error, Some(Error::Encoding(message)) if message.starts_with("time sync:"))
            );
        }
    }
}

#[test]
fn global_coalescing_bounds_direct_routed_and_sc_vmac_without_extending_cadence() {
    let limiter = TimeSyncLimiter::new(TimeSyncPolicy {
        global_coalesce_window: Duration::from_secs(2),
        ..Default::default()
    });
    let contexts = [
        received(&[1], None),
        received(&[1], Some((7, &[8]))),
        received(&[2, 3, 4, 5, 6, 7], None),
    ];
    let now = runtime_clock::now();
    assert!(limiter
        .apply_at(&contexts[0], None, now, || Err(denied("step cap exceeded")))
        .is_err());
    for (index, context) in contexts.iter().enumerate() {
        let at = now + Duration::from_secs(index as u64 * 2);
        limiter.apply_at(context, None, at, || Ok(())).unwrap();
        for other in &contexts {
            assert!(limiter
                .apply_at(other, None, at + Duration::from_millis(1999), || panic!(
                    "globally coalesced"
                ))
                .is_err());
        }
    }
    assert!(limiter.state.lock().unwrap().sources.is_empty());
}

/// One node gets one per-source budget whichever form its requests take:
/// relayed with this network's own number as SNET, a request comes from the
/// station its SADR names, the same source as one from that MAC with no
/// SNET (#1458). With the number unknown, or on another network, the
/// relayed form is a source of its own.
#[test]
fn a_station_shares_one_budget_direct_and_relayed_through_this_network() {
    const THIS_NETWORK: u16 = 7;
    let station: &[u8] = &[10, 0, 0, 3, 0xBA, 0xC0];
    let router: &[u8] = &[10, 0, 0, 9, 0xBA, 0xC0];
    let direct = received(station, None);
    for (relayed_on, number, shared) in [
        (THIS_NETWORK, Some(THIS_NETWORK), true),
        (THIS_NETWORK, None, false),
        (THIS_NETWORK + 1, Some(THIS_NETWORK), false),
    ] {
        let limiter = TimeSyncLimiter::new(TimeSyncPolicy {
            per_source_rate: Some(rate(1)),
            ..Default::default()
        });
        let now = runtime_clock::now();
        limiter.apply_at(&direct, number, now, || Ok(())).unwrap();
        let relayed = received(router, Some((relayed_on, station)));
        let second = limiter.apply_at(&relayed, number, now, || Ok(()));
        assert_eq!(second.is_err(), shared, "{relayed_on} {number:?}");
    }
}
