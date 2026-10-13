//! Time synchronization handler controls.
//!
//! The async tests run on tokio's paused clock, which the time-sync limiter
//! reads ([`runtime_clock::now`], #1550). Time moves only when a test
//! advances it, so its windows close at exact instants and a stalled runner
//! can't close one under a test.
use super::*;
use crate::server::test_transport::{SendMode, StartMode, TestTransport};
use bacnet_network::layer::ReceivedApdu;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::port::TransportProvenance;
use bacnet_types::primitives::{Date, Time};
use std::sync::atomic::AtomicUsize;

/// Any send panics: time sync must be silent on the wire. Without an inbound
/// link, startup panics too.
fn silent(incoming: Option<mpsc::Receiver<ReceivedNpdu>>) -> TestTransport {
    let builder = TestTransport::builder()
        .local_mac(&[99])
        .unicast(SendMode::Panic("time sync must be silent on wire"))
        .broadcast(SendMode::Panic("time sync must be silent on wire"));
    match incoming {
        Some(incoming) => builder.inbound(incoming),
        None => builder.start(StartMode::Panic("silent transport has no inbound link")),
    }
    .build()
}

fn date() -> Date {
    Date {
        year: 124,
        month: 7,
        day: 4,
        day_of_week: 4,
    }
}
fn time(hour: u8) -> Time {
    Time {
        hour,
        minute: 0,
        second: 0,
        hundredths: 0,
    }
}
fn encoded(hour: u8) -> Bytes {
    let mut bytes = BytesMut::new();
    TimeSynchronizationRequest {
        date: date(),
        time: time(hour),
    }
    .encode(&mut bytes);
    bytes.freeze()
}
fn received(kind: u8) -> ReceivedApdu {
    ReceivedApdu {
        direct_response: None,
        apdu: Bytes::new(),
        source_mac: MacAddr::from_slice(if kind == 2 { &[2, 3, 4, 5, 6, 7] } else { &[1] }),
        ingress_network: None,
        source_network: (kind == 1).then(|| NpduAddress {
            network: 7,
            mac_address: MacAddr::from_slice(&[8]),
        }),
        link_layer_group: false,
        is_group: false,
        global_broadcast: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}
fn clock() -> Arc<ServerClock> {
    let clock = Arc::new(ServerClock::new(ClockConfig::new(300, true).unwrap()));
    clock.synchronize(date(), time(9), false).unwrap();
    clock
}
fn config(policy: TimeSyncPolicy, callbacks: &Arc<AtomicUsize>) -> ServerConfig {
    let callbacks = callbacks.clone();
    ServerConfig {
        time_sync_policy: policy,
        on_time_sync: Some(Arc::new(move |_| {
            callbacks.fetch_add(1, Ordering::SeqCst);
        })),
        ..Default::default()
    }
}
async fn dispatch(
    config: &ServerConfig,
    clock: &Arc<ServerClock>,
    limiter: &Arc<TimeSyncLimiter>,
    is_utc: bool,
    received: &ReceivedApdu,
    data: Bytes,
) {
    BACnetServer::<TestTransport>::handle_unconfirmed_request(
        &UnconfirmedServices {
            time_sync_limiter: Arc::clone(limiter),
            clock: Some(Arc::clone(clock)),
            ..UnconfirmedServices::for_test(
                Arc::new(NetworkLayer::new(silent(None))),
                config.clone(),
            )
        },
        UnconfirmedRequestPdu {
            service_choice: if is_utc {
                UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION
            } else {
                UnconfirmedServiceChoice::TIME_SYNCHRONIZATION
            },
            service_request: data,
        },
        received,
    )
    .await;
}

#[tokio::test(start_paused = true)]
async fn time_sync_disabled_empty_allowlist_and_step_denials_leave_clock_and_observer_untouched() {
    for policy in [
        TimeSyncPolicy {
            enabled: false,
            ..Default::default()
        },
        TimeSyncPolicy {
            source_restriction: Some(TimeSyncSourceRestriction::new(vec![]).unwrap()),
            ..Default::default()
        },
        TimeSyncPolicy {
            max_step: Some(Duration::from_secs(60)),
            ..Default::default()
        },
    ] {
        for kind in 0..3 {
            for is_utc in [false, true] {
                let clock = clock();
                let calls = Arc::new(AtomicUsize::new(0));
                let config = config(policy.clone(), &calls);
                let limiter = Arc::new(TimeSyncLimiter::new(policy.clone()));
                let context = received(kind);
                // Both forward and backward corrections exceed the cap.
                for local_hour in [8, 10] {
                    dispatch(
                        &config,
                        &clock,
                        &limiter,
                        is_utc,
                        &context,
                        encoded(local_hour + if is_utc { 4 } else { 0 }),
                    )
                    .await;
                    let frame = clock.read_clock().unwrap();
                    assert_eq!(frame.local_date, date());
                    assert_eq!((frame.local_time.hour, frame.local_time.minute), (9, 0));
                    assert_eq!(calls.load(Ordering::SeqCst), 0);
                }
            }
        }
    }
}

#[tokio::test(start_paused = true)]
async fn time_sync_allowlist_accepts_exact_direct_routed_and_sc_vmac_with_callback_provenance() {
    for kind in 0..3 {
        for is_utc in [false, true] {
            let clock = clock();
            let context = received(kind);
            let source = match &context.source_network {
                Some(source) => TimeSyncSource::Routed {
                    network: source.network,
                    address: source.mac_address.to_vec(),
                },
                None => TimeSyncSource::Direct(context.source_mac.to_vec()),
            };
            let policy = TimeSyncPolicy {
                source_restriction: Some(TimeSyncSourceRestriction::new(vec![source]).unwrap()),
                ..Default::default()
            };
            let expected_mac = context.source_mac.clone();
            let expected_route = context.source_network.clone();
            let data = encoded(if is_utc { 14 } else { 10 });
            let expected_data = data.clone();
            let seen = Arc::new(AtomicUsize::new(0));
            let seen_callback = seen.clone();
            let observed_clock = clock.clone();
            let config = ServerConfig {
                time_sync_policy: policy.clone(),
                on_time_sync: Some(Arc::new(move |event| {
                    assert_eq!(event.source_mac, expected_mac);
                    assert_eq!(event.source_network, expected_route);
                    assert_eq!(event.raw_service_data, expected_data);
                    assert_eq!(event.is_utc, is_utc);
                    assert_eq!(observed_clock.read_clock().unwrap().local_time.hour, 10);
                    seen_callback.fetch_add(1, Ordering::SeqCst);
                })),
                ..Default::default()
            };
            let limiter = Arc::new(TimeSyncLimiter::new(policy));
            let mut other = received(kind);
            if let Some(source) = &mut other.source_network {
                source.network += 1;
            } else {
                other.source_mac = MacAddr::from_slice(&[42]);
            }
            dispatch(&config, &clock, &limiter, is_utc, &other, data.clone()).await;
            assert_eq!(seen.load(Ordering::SeqCst), 0);
            assert_eq!(clock.read_clock().unwrap().local_time.hour, 9);
            dispatch(&config, &clock, &limiter, is_utc, &context, data).await;
            assert_eq!(seen.load(Ordering::SeqCst), 1);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn time_sync_malformed_denied_sources_and_oversteps_do_not_starve_valid_requests() {
    let policy = TimeSyncPolicy {
        source_restriction: Some(
            TimeSyncSourceRestriction::new(vec![TimeSyncSource::Direct(vec![1])]).unwrap(),
        ),
        max_step: Some(Duration::from_secs(3600)),
        per_source_rate: Some(TimeSyncRateLimit {
            max_per_second: 1.0,
            burst_capacity: 1,
        }),
        global_rate: Some(TimeSyncRateLimit {
            max_per_second: 1.0,
            burst_capacity: 1,
        }),
        coalesce_window: Duration::from_secs(60),
        ..Default::default()
    };
    let clock = clock();
    let calls = Arc::new(AtomicUsize::new(0));
    let config = config(policy.clone(), &calls);
    let limiter = Arc::new(TimeSyncLimiter::new(policy));
    for is_utc in [false, true] {
        dispatch(
            &config,
            &clock,
            &limiter,
            is_utc,
            &received(0),
            Bytes::from_static(&[0xff]),
        )
        .await;
        let mut invalid_date = encoded(10).to_vec();
        invalid_date[1] = Date::UNSPECIFIED;
        dispatch(
            &config,
            &clock,
            &limiter,
            is_utc,
            &received(0),
            invalid_date.into(),
        )
        .await;
        dispatch(&config, &clock, &limiter, is_utc, &received(2), encoded(10)).await;
        dispatch(&config, &clock, &limiter, is_utc, &received(0), encoded(20)).await;
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    dispatch(&config, &clock, &limiter, false, &received(0), encoded(10)).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // UTC and local share rate/coalescing state. A second valid correction drops.
    dispatch(&config, &clock, &limiter, true, &received(0), encoded(15)).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(clock.read_clock().unwrap().local_time.hour, 10);
}

#[tokio::test(start_paused = true)]
async fn time_sync_panicking_observer_keeps_change_and_live_ingress_continues() {
    let (tx, rx) = mpsc::channel(8);
    let (observed, mut observations) = mpsc::unbounded_channel();
    let config = ServerConfig {
        on_time_sync: Some(Arc::new(move |event| {
            observed.send(event).unwrap();
            panic!("test observer failure");
        })),
        ..Default::default()
    };
    let mut server = BACnetServer::start(config, ObjectDatabase::new(), silent(Some(rx)))
        .await
        .unwrap();
    for (is_utc, hour) in [(false, 10), (true, 11), (false, 12)] {
        let mut npdu = BytesMut::from(&[1, 0][..]);
        encode_apdu(
            &mut npdu,
            &Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
                service_choice: if is_utc {
                    UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION
                } else {
                    UnconfirmedServiceChoice::TIME_SYNCHRONIZATION
                },
                service_request: encoded(hour),
            }),
        )
        .unwrap();
        tx.send(ReceivedNpdu {
            direct_response: None,
            npdu: npdu.freeze(),
            source_mac: MacAddr::from_slice(&[2, 3, 4, 5, 6, 7]),
            link_layer_group: false,
            data_attributes: Vec::new(),
            provenance: TransportProvenance::unverified(),
            reply_tx: None,
        })
        .await
        .unwrap();
        let event = tokio::time::timeout(Duration::from_secs(2), observations.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.is_utc, is_utc);
        assert_eq!(
            server
                ._clock
                .as_ref()
                .unwrap()
                .read_clock()
                .unwrap()
                .local_time
                .hour,
            hour
        );
    }
    server.stop().await.unwrap();
}

#[test]
fn time_sync_callback_unwind_is_contained_without_poisoning_limiter() {
    let clock = clock();
    let config = ServerConfig {
        on_time_sync: Some(Arc::new(|_| panic!("test observer failure"))),
        ..Default::default()
    };
    let limiter = TimeSyncLimiter::new(config.time_sync_policy.clone());
    for (is_utc, local_hour) in [(false, 10), (true, 11)] {
        apply_time_sync_request(
            Some(&clock),
            &config,
            &limiter,
            encoded(local_hour + if is_utc { 4 } else { 0 }),
            is_utc,
            &received(2),
            None,
        )
        .unwrap();
        assert_eq!(clock.read_clock().unwrap().local_time.hour, local_hour);
    }
}

/// Time synchronization at 10:00 from link MAC `mac`, relayed with `source`
/// as SNET/SADR when one is given, to a server that knows `number` as its
/// network's and lists only `entry`. Whether the clock took it.
async fn synced_from(
    entry: TimeSyncSource,
    number: Option<u16>,
    mac: &[u8],
    source: Option<NpduAddress>,
) -> bool {
    use bacnet_types::network_number::NetworkNumber;
    let policy = TimeSyncPolicy {
        source_restriction: Some(TimeSyncSourceRestriction::new(vec![entry]).unwrap()),
        ..Default::default()
    };
    let network = Arc::new(NetworkLayer::new(silent(None)));
    if let Some(number) = number {
        network
            .local_network_number()
            .publish(NetworkNumber::configured(number).unwrap());
    }
    let clock = clock();
    let mut received = received(0);
    received.source_mac = MacAddr::from_slice(mac);
    received.source_network = source;
    BACnetServer::<TestTransport>::handle_unconfirmed_request(
        &UnconfirmedServices {
            time_sync_limiter: Arc::new(TimeSyncLimiter::new(policy.clone())),
            clock: Some(Arc::clone(&clock)),
            ..UnconfirmedServices::for_test(network, config(policy, &Arc::new(AtomicUsize::new(0))))
        },
        UnconfirmedRequestPdu {
            service_choice: UnconfirmedServiceChoice::TIME_SYNCHRONIZATION,
            service_request: encoded(10),
        },
        &received,
    )
    .await;
    match clock.read_clock().unwrap().local_time.hour {
        10 => true,
        9 => false,
        hour => panic!("the clock moved to {hour}:00"),
    }
}

/// A routed allowlist entry on the server's own network number also names
/// the station's direct requests once that number is known (#1458); only
/// the routed entry widens.
#[tokio::test(start_paused = true)]
async fn time_sync_routed_entry_on_this_network_names_the_stations_direct_request() {
    const THIS_NETWORK: u16 = 7;
    let (station, other_station, router) = ([10, 0, 0, 3], [10, 0, 0, 4], [10, 0, 0, 9]);
    let routed_entry = || TimeSyncSource::Routed {
        network: THIS_NETWORK,
        address: station.to_vec(),
    };
    let relayed = |network| {
        Some(NpduAddress {
            network,
            mac_address: MacAddr::from_slice(&station),
        })
    };
    let known = Some(THIS_NETWORK);
    assert!(synced_from(routed_entry(), known, &station, None).await);
    assert!(synced_from(routed_entry(), known, &router, relayed(THIS_NETWORK)).await);
    // Unknown number, another number, another station: no direct match.
    assert!(!synced_from(routed_entry(), None, &station, None).await);
    assert!(!synced_from(routed_entry(), Some(THIS_NETWORK + 1), &station, None).await);
    assert!(!synced_from(routed_entry(), known, &other_station, None).await);
    // A direct entry still names no routed claim of the same station.
    let direct = || TimeSyncSource::Direct(station.to_vec());
    assert!(synced_from(direct(), known, &station, None).await);
    assert!(!synced_from(direct(), known, &router, relayed(THIS_NETWORK)).await);
}

/// Each window the limiter keeps closes exactly at its length (#1550): a
/// repeat 1 ms inside a coalescing window, or 1 ms before a rate bucket earns
/// its next token, is refused and leaves the clock alone; one at the edge is
/// taken. The request path reads the clock itself, so this needs the limiter
/// on tokio's paused clock.
#[tokio::test(start_paused = true)]
async fn time_sync_windows_close_exactly_at_their_length() {
    let rate = Some(TimeSyncRateLimit {
        max_per_second: 1.0,
        burst_capacity: 1,
    });
    let window = Duration::from_secs(2);
    let token = Duration::from_secs(1);
    // The global rate goes first, while the paused clock still reads the
    // instant the runtime started: a global bucket built on the system clock,
    // which has run on since, would then hold its next token back past the
    // edge.
    let cases = [
        (
            "global rate",
            TimeSyncPolicy {
                global_rate: rate,
                ..Default::default()
            },
            token,
        ),
        (
            "source rate",
            TimeSyncPolicy {
                per_source_rate: rate,
                ..Default::default()
            },
            token,
        ),
        (
            "source coalescing",
            TimeSyncPolicy {
                coalesce_window: window,
                ..Default::default()
            },
            window,
        ),
        (
            "global coalescing",
            TimeSyncPolicy {
                global_coalesce_window: window,
                ..Default::default()
            },
            window,
        ),
    ];
    for (name, policy, edge) in cases {
        let clock = clock();
        let calls = Arc::new(AtomicUsize::new(0));
        let config = config(policy.clone(), &calls);
        let limiter = Arc::new(TimeSyncLimiter::new(policy));
        let synced = || {
            (
                calls.load(Ordering::SeqCst),
                clock.read_clock().unwrap().local_time.hour,
            )
        };
        dispatch(&config, &clock, &limiter, false, &received(0), encoded(10)).await;
        assert_eq!(synced(), (1, 10), "{name}: first request");

        tokio::time::advance(edge - Duration::from_millis(1)).await;
        dispatch(&config, &clock, &limiter, false, &received(0), encoded(11)).await;
        assert_eq!(synced(), (1, 10), "{name}: 1 ms inside");

        tokio::time::advance(Duration::from_millis(1)).await;
        dispatch(&config, &clock, &limiter, false, &received(0), encoded(12)).await;
        assert_eq!(synced(), (2, 12), "{name}: at the edge");
    }
}
