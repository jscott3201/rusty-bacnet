//! Unconfirmed-service dispatch (Who-Is, Who-Has, time sync, text, and Audit
//! notification receipt) — see `EXECUTED_UNCONFIRMED`.
//!
//! Split out of `requests.rs` to keep every file under the 700-LOC cap.

use super::super::*;
use bacnet_objects::clock::ClockReader;
use bacnet_services::device_mgmt::TimeSynchronizationRequest;
use bacnet_services::who_has::{WhoHasObject, WhoHasRequest};
use std::panic::{catch_unwind, AssertUnwindSafe};

#[cfg(test)]
/// Every unconfirmed service choice with an inbound execution arm in
/// `handle_unconfirmed_request` below. Keep in lockstep with the dispatch
/// chain — see `EXECUTED_CONFIRMED` in `requests/mod.rs` for the cross-check
/// contract.
pub(crate) const EXECUTED_UNCONFIRMED: &[UnconfirmedServiceChoice] = &[
    UnconfirmedServiceChoice::WHO_IS,
    UnconfirmedServiceChoice::WHO_HAS,
    UnconfirmedServiceChoice::TIME_SYNCHRONIZATION,
    UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION,
    UnconfirmedServiceChoice::UNCONFIRMED_TEXT_MESSAGE,
    UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION,
];

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Handle an unconfirmed request (e.g., WhoIs).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::server) async fn handle_unconfirmed_request(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        config: &ServerConfig,
        clock: Option<&Arc<ServerClock>>,
        comm_state: &Arc<AtomicU8>,
        device_bindings: &Arc<RwLock<DeviceBindingTable>>,
        discovery_limiter: &Arc<DiscoveryLimiter>,
        time_sync_limiter: &Arc<TimeSyncLimiter>,
        req: UnconfirmedRequestPdu,
        received: &bacnet_network::layer::ReceivedApdu,
    ) {
        let comm = comm_state.load(Ordering::Acquire);
        if comm == 1 {
            tracing::debug!("Dropping unconfirmed service: DCC is DISABLE");
            return;
        }

        if req.service_choice == UnconfirmedServiceChoice::I_AM {
            let i_am = match IAmRequest::decode(&req.service_request) {
                Ok(request) => request,
                Err(_) => {
                    debug!("Ignoring malformed I-Am observation");
                    return;
                }
            };
            let outcome = device_bindings.write().await.observe_i_am_at(
                i_am.object_identifier,
                &received.source_mac,
                received.source_network.as_ref(),
                Instant::now(),
                |mac| network.transport().is_broadcast_mac(mac),
            );
            match outcome {
                device_bindings::ObservationOutcome::RejectedInvalid => {
                    debug!("Ignoring unusable I-Am observation");
                }
                device_bindings::ObservationOutcome::RejectedCapacity => {
                    warn!("Device binding capacity reached; I-Am observation ignored");
                }
                device_bindings::ObservationOutcome::Inserted
                | device_bindings::ObservationOutcome::Refreshed
                | device_bindings::ObservationOutcome::ConfiguredPreserved => {}
            }
        } else if req.service_choice == UnconfirmedServiceChoice::WHO_IS {
            let who_is = match WhoIsRequest::decode(&req.service_request) {
                Ok(r) => r,
                Err(e) => {
                    warn!(error = %e, "Failed to decode WhoIs");
                    return;
                }
            };

            let db = db.read().await;
            let device_oid = db
                .list_objects()
                .into_iter()
                .find(|oid| oid.object_type() == ObjectType::DEVICE);

            if let Some(device_oid) = device_oid {
                let instance = device_oid.instance_number();

                let in_range = match (who_is.low_limit, who_is.high_limit) {
                    (Some(low), Some(high)) => instance >= low && instance <= high,
                    _ => true,
                };

                if in_range {
                    let i_am = IAmRequest {
                        object_identifier: device_oid,
                        max_apdu_length: config.max_apdu_length,
                        segmentation_supported: config.segmentation_supported,
                        vendor_id: config.vendor_id,
                    };

                    let mut service_buf = BytesMut::new();
                    i_am.encode(&mut service_buf);

                    let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
                        service_choice: UnconfirmedServiceChoice::I_AM,
                        service_request: service_buf.freeze(),
                    });

                    let mut buf = BytesMut::new();
                    encode_apdu(&mut buf, &pdu).expect("valid APDU encoding");

                    let now = Instant::now();
                    let is_unicast = !received.is_group && !received.link_layer_group;
                    let (res, directed) = if let Some(ref source_net) = received.source_network {
                        (
                            network
                                .send_apdu_routed(
                                    &buf,
                                    source_net.network,
                                    &source_net.mac_address,
                                    &received.source_mac,
                                    false,
                                    NetworkPriority::NORMAL,
                                )
                                .await,
                            true,
                        )
                    } else if config.discovery_policy.prefer_directed_responses || is_unicast {
                        (
                            network
                                .send_apdu(
                                    &buf,
                                    &received.source_mac,
                                    false,
                                    NetworkPriority::NORMAL,
                                )
                                .await,
                            true,
                        )
                    } else {
                        (
                            network
                                .broadcast_apdu(&buf, false, NetworkPriority::NORMAL)
                                .await,
                            false,
                        )
                    };

                    if let Err(e) = res {
                        warn!(error = %e, directed, "Failed to send IAm");
                    } else {
                        discovery_limiter.record_i_am_sent(
                            buf.len(),
                            directed,
                            &received.source_mac,
                            received.source_network.as_ref(),
                            now,
                        );
                    }
                }
            }
        } else if req.service_choice == UnconfirmedServiceChoice::WHO_HAS {
            let who_has = match WhoHasRequest::decode(&req.service_request) {
                Ok(r) => r,
                Err(e) => {
                    warn!(error = %e, "Failed to decode WhoHas");
                    return;
                }
            };

            let target = match &who_has.object {
                WhoHasObject::Identifier(oid) => WhoHasTarget::Id(*oid),
                WhoHasObject::Name(name) => WhoHasTarget::Name(name.clone()),
            };

            let now = Instant::now();
            if discovery_limiter.is_negative_who_has(&target, now) {
                return;
            }

            let db = db.read().await;
            let device_oid = db
                .list_objects()
                .into_iter()
                .find(|oid| oid.object_type() == ObjectType::DEVICE);

            if let Some(device_oid) = device_oid {
                match handlers::handle_who_has(&db, &req.service_request, device_oid) {
                    Ok(Some(i_have)) => {
                        let mut service_buf = BytesMut::new();
                        if let Err(e) = i_have.encode(&mut service_buf) {
                            warn!(error = %e, "Failed to encode IHave");
                        } else {
                            let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
                                service_choice: UnconfirmedServiceChoice::I_HAVE,
                                service_request: service_buf.freeze(),
                            });

                            let mut buf = BytesMut::new();
                            encode_apdu(&mut buf, &pdu).expect("valid APDU encoding");

                            if !discovery_limiter.try_consume_who_has_response(
                                &target,
                                received,
                                buf.len(),
                                now,
                            ) {
                                return;
                            }

                            let is_unicast = !received.is_group && !received.link_layer_group;
                            let (res, directed) =
                                if let Some(ref source_net) = received.source_network {
                                    (
                                        network
                                            .send_apdu_routed(
                                                &buf,
                                                source_net.network,
                                                &source_net.mac_address,
                                                &received.source_mac,
                                                false,
                                                NetworkPriority::NORMAL,
                                            )
                                            .await,
                                        true,
                                    )
                                } else if config.discovery_policy.prefer_directed_responses
                                    || is_unicast
                                {
                                    (
                                        network
                                            .send_apdu(
                                                &buf,
                                                &received.source_mac,
                                                false,
                                                NetworkPriority::NORMAL,
                                            )
                                            .await,
                                        true,
                                    )
                                } else {
                                    (
                                        network
                                            .broadcast_apdu(&buf, false, NetworkPriority::NORMAL)
                                            .await,
                                        false,
                                    )
                                };

                            if let Err(e) = res {
                                warn!(error = %e, directed, "Failed to send IHave");
                            } else {
                                discovery_limiter.record_i_have_sent(
                                    buf.len(),
                                    directed,
                                    &target,
                                    &received.source_mac,
                                    received.source_network.as_ref(),
                                    now,
                                );
                            }
                        }
                    }
                    Ok(None) => {
                        discovery_limiter.record_negative_who_has(target, now);
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to decode WhoHas");
                    }
                }
            }
        } else if req.service_choice == UnconfirmedServiceChoice::TIME_SYNCHRONIZATION
            || req.service_choice == UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION
        {
            debug!("Received time synchronization request");
            let is_utc = req.service_choice == UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION;
            if let Err(error) = apply_time_sync_request(
                clock.map(Arc::as_ref),
                config,
                time_sync_limiter,
                req.service_request.clone(),
                is_utc,
                received,
            ) {
                debug!(%error, is_utc, "Ignoring time synchronization request");
            }
        } else if req.service_choice == UnconfirmedServiceChoice::UNCONFIRMED_TEXT_MESSAGE {
            match handlers::handle_text_message(&req.service_request) {
                Ok(msg) => {
                    debug!(
                        source = ?msg.source_device,
                        priority = ?msg.message_priority,
                        "UnconfirmedTextMessage: {}",
                        msg.message
                    );
                }
                Err(e) => {
                    debug!(error = %e, "UnconfirmedTextMessage decode failed");
                }
            }
        } else if req.service_choice == UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION {
            if let Err(error) = super::audit_notification::receive_unconfirmed_audit_notification(
                db,
                config,
                &received.source_mac,
                received.source_network.as_ref(),
                &req.service_request,
            )
            .await
            {
                debug!(%error, "Ignoring UnconfirmedAuditNotification request");
            }
        } else {
            debug!(
                service = req.service_choice.to_raw(),
                "Ignoring unsupported unconfirmed service"
            );
        }
    }
}

pub(super) fn apply_time_sync_request(
    clock: Option<&ServerClock>,
    config: &ServerConfig,
    limiter: &TimeSyncLimiter,
    raw_service_data: Bytes,
    is_utc: bool,
    received: &bacnet_network::layer::ReceivedApdu,
) -> Result<(), Error> {
    let request = TimeSynchronizationRequest::decode(&raw_service_data)?;
    let supplied = clock::date_time_to_hundredths(request.date, request.time)?;
    let policy = &config.time_sync_policy;
    if !policy.enabled {
        return Err(time_sync_policy::denied("disabled"));
    }
    if policy
        .source_restriction
        .as_ref()
        .is_some_and(|restriction| {
            !restriction.allows(&received.source_mac, received.source_network.as_ref())
        })
    {
        return Err(time_sync_policy::denied("source not allowed"));
    }
    let clock = clock.ok_or_else(|| Error::Encoding("Device clock is disabled".into()))?;
    let mut delta_hundredths = None;
    let result = limiter.apply_at(received, Instant::now(), || {
        delta_hundredths = clock
            .read_clock()
            .and_then(|frame| time_sync_policy::step_hundredths(supplied, is_utc, frame));
        time_sync_policy::check_step(delta_hundredths, policy.max_step)?;
        clock.synchronize(request.date, request.time, is_utc)
    });
    debug!(
        accepted = result.is_ok(),
        ?delta_hundredths,
        is_utc,
        "Time synchronization decision"
    );
    result?;

    if let Some(callback) = &config.on_time_sync {
        let data = TimeSyncData {
            raw_service_data,
            is_utc,
            source_mac: received.source_mac.clone(),
            source_network: received.source_network.clone(),
        };
        if catch_unwind(AssertUnwindSafe(|| callback(data))).is_err() {
            debug!(
                is_utc,
                "Time synchronization observer panicked after clock update"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod time_sync_tests {
    use super::*;
    use bacnet_network::layer::ReceivedApdu;
    use bacnet_transport::port::ReceivedNpdu;
    use bacnet_types::primitives::{Date, Time};
    use std::sync::atomic::AtomicUsize;

    struct SilentTransport(Option<mpsc::Receiver<ReceivedNpdu>>);
    impl TransportPort for SilentTransport {
        async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
            Ok(self.0.take().unwrap())
        }
        async fn stop(&mut self) -> Result<(), Error> {
            Ok(())
        }
        async fn send_unicast(&self, _: &[u8], _: &[u8]) -> Result<(), Error> {
            panic!("time sync must be silent on wire")
        }
        async fn send_broadcast(&self, _: &[u8]) -> Result<(), Error> {
            panic!("time sync must be silent on wire")
        }
        fn local_mac(&self) -> &[u8] {
            &[99]
        }
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
            apdu: Bytes::new(),
            source_mac: MacAddr::from_slice(if kind == 2 { &[2, 3, 4, 5, 6, 7] } else { &[1] }),
            ingress_network: None,
            source_network: (kind == 1).then(|| NpduAddress {
                network: 7,
                mac_address: MacAddr::from_slice(&[8]),
            }),
            link_layer_group: false,
            is_group: false,
            data_attributes: Vec::new(),
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
        BACnetServer::<SilentTransport>::handle_unconfirmed_request(
            &Arc::new(RwLock::new(ObjectDatabase::new())),
            &Arc::new(NetworkLayer::new(SilentTransport(None))),
            config,
            Some(clock),
            &Arc::new(AtomicU8::new(0)),
            &Arc::new(RwLock::new(DeviceBindingTable::new())),
            &Arc::new(DiscoveryLimiter::new(DiscoveryPolicy::default(), None)),
            limiter,
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

    #[tokio::test]
    async fn time_sync_disabled_empty_allowlist_and_step_denials_leave_clock_and_observer_untouched(
    ) {
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

    #[tokio::test]
    async fn time_sync_allowlist_accepts_exact_direct_routed_and_sc_vmac_with_callback_provenance()
    {
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

    #[tokio::test]
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

    #[tokio::test]
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
        let mut server =
            BACnetServer::start(config, ObjectDatabase::new(), SilentTransport(Some(rx)))
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
                npdu: npdu.freeze(),
                source_mac: MacAddr::from_slice(&[2, 3, 4, 5, 6, 7]),
                link_layer_group: false,
                data_attributes: Vec::new(),
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
            )
            .unwrap();
            assert_eq!(clock.read_clock().unwrap().local_time.hour, local_hour);
        }
    }
}
