use super::*;
use crate::server::request_admission::{Class, Rejection};

#[path = "peer_admission_tests.rs"]
mod peer_admission_tests;

async fn small_fixture() -> (
    BACnetServer<HeldTransport>,
    mpsc::Sender<ReceivedNpdu>,
    mpsc::UnboundedReceiver<oneshot::Receiver<()>>,
) {
    fixture_with_config(
        "admission",
        ServerConfig {
            request_admission_policy: RequestAdmissionPolicy {
                max_confirmed_in_flight: 1,
                max_unconfirmed_in_flight: 1,
                ..Default::default()
            },
            discovery_policy: DiscoveryPolicy::unlimited(),
            segmentation_supported: Segmentation::BOTH,
            ..ServerConfig::default()
        },
    )
    .await
}

async fn dispatch(
    server: &BACnetServer<HeldTransport>,
    apdu: Apdu,
    source: Option<NpduAddress>,
    reply_tx: Option<oneshot::Sender<Bytes>>,
) {
    BACnetServer::dispatch(
        &server.db,
        &server.network,
        &server.cov_table,
        &server.seg_ack_senders,
        &server.seg_send_permits,
        &server.cov_in_flight,
        &server.server_tsm,
        &server.notification_transactions,
        &server.confirmed_request_tracker,
        &server.device_bindings,
        &server.comm_state,
        &server.dcc_timer,
        &Arc::new(server.config.clone()),
        &server._clock,
        &server.discovery_limiter,
        &server.request_tasks,
        &[1],
        apdu,
        bacnet_network::layer::ReceivedApdu {
            apdu: Bytes::new(),
            source_mac: MacAddr::from_slice(&[1]),
            source_network: source,
            link_layer_group: false,
            is_group: false,
            data_attributes: Vec::new(),
            reply_tx,
        },
    )
    .await;
}

fn who_is() -> Apdu {
    Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
        service_choice: UnconfirmedServiceChoice::WHO_IS,
        service_request: Bytes::new(),
    })
}

async fn observed(
    started: &mut mpsc::UnboundedReceiver<oneshot::Receiver<()>>,
) -> oneshot::Receiver<()> {
    tokio::time::timeout(Duration::from_secs(2), started.recv())
        .await
        .unwrap()
        .unwrap()
}

fn request(id: u8) -> Apdu {
    let Apdu::ConfirmedRequest(mut req) = confirmed(false) else {
        unreachable!()
    };
    req.invoke_id = id;
    Apdu::ConfirmedRequest(req)
}

#[tokio::test]
async fn admission_default_confirmed_limit_rejects_before_handler() {
    let (mut server, tx, mut started) = fixture_with_config(
        "global admission",
        ServerConfig {
            request_admission_policy: RequestAdmissionPolicy {
                max_confirmed_in_flight_per_peer: 64,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .await;
    for id in 0..65 {
        inject(&tx, request(id)).await;
        tokio::time::timeout(Duration::from_secs(2), started.recv())
            .await
            .unwrap()
            .unwrap();
    }
    {
        let frames = server.network.transport().frames.lock().unwrap();
        assert_eq!(frames.len(), 65);
        assert!(
            matches!(&frames[64], Apdu::Abort(abort)
            if abort.sent_by_server && abort.invoke_id == 64
                && abort.abort_reason == AbortReason::OUT_OF_RESOURCES),
            "65th held confirmed request must not execute: {:?}",
            frames[64]
        );
    }
    server.stop().await.unwrap();
}

#[tokio::test]
async fn admission_independent_handlers_and_eight_owned_abort_workers_never_queue() {
    let (mut server, _tx, mut started) = small_fixture().await;
    dispatch(&server, request(1), None, None).await;
    let original = observed(&mut started).await;
    dispatch(&server, who_is(), None, None).await;
    let unconfirmed = observed(&mut started).await;
    dispatch(&server, who_is(), None, None).await;
    for id in 2..=9 {
        dispatch(&server, request(id), None, None).await;
        observed(&mut started).await;
    }
    dispatch(&server, request(10), None, None).await;
    let counters = server.request_admission_counters();
    assert_eq!(counters.confirmed_active, 1);
    assert_eq!(counters.unconfirmed_active, 1);
    assert_eq!(counters.confirmed_admitted_total, 1);
    assert_eq!(counters.unconfirmed_admitted_total, 1);
    assert_eq!(counters.confirmed_overloaded_total, 9);
    assert_eq!(counters.unconfirmed_overloaded_total, 1);
    assert_eq!(counters.abort_active, 8);
    assert_eq!(counters.abort_admitted_total, 8);
    assert_eq!(counters.confirmed_fallback_dropped_total, 1);
    // Real outgoing notification transactions still finish inline while both
    // handler classes and every overload response worker are saturated.
    for kind in 0..4 {
        let service_choice = ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION;
        let (operation, mut completed) = server
            .notification_transactions
            .reserve(
                crate::server::notification_transactions::canonical_direct_peer(&[1]),
                service_choice,
            )
            .unwrap();
        let invoke_id = operation.invoke_id();
        let terminal = match kind {
            0 => Apdu::SimpleAck(SimpleAck {
                invoke_id,
                service_choice,
            }),
            1 => Apdu::Error(ErrorPdu {
                invoke_id,
                service_choice,
                error_class: ErrorClass::SERVICES,
                error_code: ErrorCode::OTHER,
                error_data: Bytes::new(),
            }),
            2 => Apdu::Reject(RejectPdu {
                invoke_id,
                reject_reason: RejectReason::OTHER,
            }),
            _ => Apdu::Abort(AbortPdu {
                invoke_id,
                sent_by_server: false,
                abort_reason: AbortReason::OTHER,
            }),
        };
        tokio::time::timeout(
            Duration::from_secs(2),
            dispatch(&server, terminal, None, None),
        )
        .await
        .unwrap();
        assert_eq!(
            completed.try_recv(),
            Ok(if kind == 0 {
                CovAckResult::Ack
            } else {
                CovAckResult::Error
            })
        );
        drop(operation);
    }
    assert_eq!(server.request_admission_counters(), counters);
    assert!(started.try_recv().is_err());
    assert_eq!(server.network.transport().frames.lock().unwrap().len(), 10);
    server.network.transport().release.notify_waiters();
    original.await.unwrap();
    unconfirmed.await.unwrap();
    wait_reaped(&server).await;
    // Denied work never starts after release. All ten prior frames are final.
    assert_eq!(server.network.transport().frames.lock().unwrap().len(), 10);
    assert_eq!(server.request_admission_counters().confirmed_active, 0);
    assert_eq!(server.request_admission_counters().unconfirmed_active, 0);
    assert_eq!(server.request_admission_counters().abort_active, 0);
    dispatch(&server, request(10), None, None).await;
    observed(&mut started).await;
    assert_eq!(
        server.request_admission_counters().confirmed_admitted_total,
        2
    );
    server.stop().await.unwrap();
    assert_eq!(server.request_admission_counters().confirmed_active, 0);
}

#[tokio::test]
async fn admission_pending_and_completed_duplicates_at_capacity_have_no_abort() {
    let (mut server, _tx, mut started) = small_fixture().await;
    dispatch(&server, request(1), None, None).await;
    let original = observed(&mut started).await;
    // Exact duplicate detection already works before another task is polled.
    dispatch(&server, request(1), None, None).await;
    assert_eq!(
        server
            .request_admission_counters()
            .confirmed_overloaded_total,
        0
    );
    server.network.transport().release.notify_one();
    original.await.unwrap();
    wait_reaped(&server).await;
    dispatch(&server, request(2), None, None).await;
    observed(&mut started).await;
    dispatch(&server, request(1), None, None).await;
    dispatch(&server, request(2), None, None).await;
    assert_eq!(
        server.request_admission_counters().confirmed_admitted_total,
        2
    );
    assert_eq!(
        server
            .request_admission_counters()
            .confirmed_overloaded_total,
        0
    );
    assert_eq!(server.request_admission_counters().abort_admitted_total, 0);
    assert_eq!(server.network.transport().frames.lock().unwrap().len(), 2);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn admission_abort_reply_channel_preserves_routed_npdu_and_wire_fields() {
    let (mut server, _tx, mut started) = small_fixture().await;
    dispatch(&server, request(1), None, None).await;
    observed(&mut started).await;
    for source in [
        None,
        Some(NpduAddress {
            network: 42,
            mac_address: MacAddr::from_slice(&[7]),
        }),
    ] {
        let (tx, rx) = oneshot::channel();
        dispatch(&server, request(2), source.clone(), Some(tx)).await;
        let wire = tokio::time::timeout(Duration::from_secs(2), rx)
            .await
            .unwrap()
            .unwrap();
        let npdu = decode_npdu(wire).unwrap();
        assert_eq!(npdu.destination, source);
        assert!(!npdu.expecting_reply);
        assert!(
            matches!(apdu::decode_apdu(npdu.payload).unwrap(), Apdu::Abort(a)
            if a.sent_by_server && a.invoke_id == 2 && a.abort_reason == AbortReason::OUT_OF_RESOURCES)
        );
    }
    assert_eq!(server.network.transport().frames.lock().unwrap().len(), 1);
    server.stop().await.unwrap();
    assert_eq!(server.request_admission_counters().abort_active, 0);
}

#[tokio::test]
async fn admission_stop_seals_classes_without_overload_and_joins_abort_workers() {
    let (mut server, _tx, mut started) = small_fixture().await;
    dispatch(&server, request(1), None, None).await;
    let mut original = observed(&mut started).await;
    dispatch(&server, request(2), None, None).await;
    let mut abort = observed(&mut started).await;
    server.stop().await.unwrap();
    assert_eq!(original.try_recv(), Ok(()));
    assert_eq!(abort.try_recv(), Ok(()));
    dispatch(&server, request(3), None, None).await;
    dispatch(&server, who_is(), None, None).await;
    let c = server.request_admission_counters();
    assert_eq!(
        c.confirmed_active + c.unconfirmed_active + c.abort_active,
        0
    );
    assert_eq!(c.confirmed_overloaded_total, 1);
    assert_eq!(c.unconfirmed_overloaded_total, 0);
    assert_eq!(c.confirmed_shutdown_rejected_total, 1);
    assert_eq!(c.unconfirmed_shutdown_rejected_total, 1);
}

#[tokio::test]
async fn admission_panic_releases_real_handler_and_abort_and_allows_retry() {
    let (mut server, tx, mut started) = small_fixture().await;
    for id in [1, 1] {
        inject(&tx, request(id)).await;
        let released = observed(&mut started).await;
        server
            .network
            .transport()
            .panic_next
            .store(true, Ordering::Release);
        server.network.transport().release.notify_one();
        released.await.unwrap();
        wait_reaped(&server).await;
        assert_eq!(server.request_admission_counters().confirmed_active, 0);
    }
    dispatch(&server, request(3), None, None).await;
    observed(&mut started).await;
    dispatch(&server, request(4), None, None).await;
    observed(&mut started).await;
    // Cancellation, including never-polled futures, is owned by the same set.
    server.stop().await.unwrap();
    assert_eq!(server.request_admission_counters().abort_active, 0);
}

#[tokio::test]
async fn admission_guards_not_joinset_length_and_closed_not_overload() {
    let owner = crate::server::request_tasks::RequestTasks::new(RequestAdmissionPolicy {
        max_confirmed_in_flight: 1,
        max_unconfirmed_in_flight: 1,
        ..Default::default()
    })
    .unwrap();
    for class in [Class::Confirmed, Class::Unconfirmed, Class::Abort] {
        let (tx, rx) = oneshot::channel();
        owner
            .try_spawn(
                class,
                crate::server::request_peer::canonical_requester(&[1], None),
                || async move {
                    tx.send(()).unwrap();
                },
            )
            .unwrap();
        rx.await.unwrap();
        // One yield lets the guard drop, but deliberately never reap the set.
        tokio::task::yield_now().await;
        owner
            .try_spawn(
                class,
                crate::server::request_peer::canonical_requester(&[1], None),
                || async {},
            )
            .unwrap();
    }
    owner.close();
    while !owner.is_empty() {
        owner.join_next().await;
    }
    for class in [Class::Confirmed, Class::Unconfirmed, Class::Abort] {
        assert_eq!(
            owner.try_spawn(
                class,
                crate::server::request_peer::canonical_requester(&[1], None),
                || async { panic!("closed task ran") }
            ),
            Err(Rejection::Closed)
        );
    }
    let c = owner.counters();
    assert_eq!(c.confirmed_admitted_total, 2);
    assert_eq!(
        c.confirmed_overloaded_total
            + c.unconfirmed_overloaded_total
            + c.confirmed_fallback_dropped_total,
        0
    );
    assert_eq!(
        c.confirmed_active + c.unconfirmed_active + c.abort_active,
        0
    );
}

#[test]
fn admission_policy_validates_zero_upper_bound_and_defaults() {
    let defaults = RequestAdmissionPolicy::default();
    assert_eq!(
        (
            defaults.max_confirmed_in_flight,
            defaults.max_unconfirmed_in_flight
        ),
        (64, 32)
    );
    for bad in [0, Semaphore::MAX_PERMITS + 1, usize::MAX] {
        for policy in [
            RequestAdmissionPolicy {
                max_confirmed_in_flight: bad,
                ..defaults
            },
            RequestAdmissionPolicy {
                max_unconfirmed_in_flight: bad,
                ..defaults
            },
        ] {
            assert!(policy.validate().is_err());
        }
    }
    RequestAdmissionPolicy {
        max_confirmed_in_flight: Semaphore::MAX_PERMITS,
        max_unconfirmed_in_flight: 1,
        ..Default::default()
    }
    .validate()
    .unwrap();
}

#[tokio::test]
async fn admission_direct_and_routed_abort_send_release_on_error_and_panic() {
    let (mut server, _tx, mut started) = small_fixture().await;
    let db = Arc::clone(&server.db);
    let held_db = db.write().await;
    dispatch(&server, request(1), None, None).await;
    let source = NpduAddress {
        network: 42,
        mac_address: MacAddr::from_slice(&[7]),
    };
    for (id, route, panic) in [(2, None, false), (3, Some(source.clone()), true)] {
        dispatch(&server, request(id), route.clone(), None).await;
        let released = observed(&mut started).await;
        assert_eq!(
            server
                .network
                .transport()
                .routes
                .lock()
                .unwrap()
                .last()
                .unwrap(),
            &(route, MacAddr::from_slice(&[1]))
        );
        server
            .network
            .transport()
            .panic_next
            .store(panic, Ordering::Release);
        server
            .network
            .transport()
            .fail_next
            .store(!panic, Ordering::Release);
        server.network.transport().release.notify_one();
        released.await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while server.request_admission_counters().abort_active != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(server.request_admission_counters().confirmed_active, 1);
    }
    assert_eq!(server.request_admission_counters().abort_admitted_total, 2);
    drop(held_db);
    server.stop().await.unwrap();
    assert_eq!(server.request_admission_counters().abort_active, 0);
}

#[tokio::test]
async fn admission_dcc_prechecks_and_duplicate_before_first_poll() {
    let (mut server, _tx, mut started) = small_fixture().await;
    dispatch(&server, request(1), None, None).await;
    dispatch(&server, request(1), None, None).await;
    assert_eq!(
        server.request_admission_counters().confirmed_admitted_total,
        1
    );
    assert_eq!(
        server
            .request_admission_counters()
            .confirmed_overloaded_total,
        0
    );
    observed(&mut started).await;
    server.comm_state.store(1, Ordering::Release);
    dispatch(&server, request(2), None, None).await;
    dispatch(&server, who_is(), None, None).await;
    assert_eq!(
        server
            .request_admission_counters()
            .confirmed_overloaded_total,
        0
    );
    assert_eq!(
        server
            .request_admission_counters()
            .unconfirmed_admitted_total,
        0
    );
    assert_eq!(server.request_admission_counters().abort_active, 0);
    server.stop().await.unwrap();
}

struct NeverStart(Arc<AtomicBool>);
impl TransportPort for NeverStart {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.0.store(true, Ordering::Release);
        panic!("invalid admission must not start transport");
    }
    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }
    async fn send_unicast(&self, _: &[u8], _: &[u8]) -> Result<(), Error> {
        unreachable!()
    }
    async fn send_broadcast(&self, _: &[u8]) -> Result<(), Error> {
        unreachable!()
    }
    fn local_mac(&self) -> &[u8] {
        &[1]
    }
}

#[tokio::test]
async fn admission_invalid_direct_generic_bip_before_transport_start() {
    for bad in [0, usize::MAX] {
        let policy = RequestAdmissionPolicy {
            max_confirmed_in_flight: bad,
            max_unconfirmed_in_flight: 1,
            ..Default::default()
        };
        let started = Arc::new(AtomicBool::new(false));
        let error = BACnetServer::start(
            ServerConfig {
                request_admission_policy: policy,
                ..Default::default()
            },
            ObjectDatabase::new(),
            NeverStart(Arc::clone(&started)),
        )
        .await
        .err()
        .unwrap();
        assert!(matches!(error, Error::Encoding(m) if m.contains("max_confirmed_in_flight")));
        let error = BACnetServer::generic_builder()
            .transport(NeverStart(Arc::clone(&started)))
            .request_admission_policy(policy)
            .build()
            .await
            .err()
            .unwrap();
        assert!(matches!(error, Error::Encoding(m) if m.contains("max_confirmed_in_flight")));
        assert!(!started.load(Ordering::Acquire));
        let error = BACnetServer::bip_builder()
            .interface(Ipv4Addr::LOCALHOST)
            .port(0)
            .request_admission_policy(policy)
            .build()
            .await
            .err()
            .unwrap();
        assert!(matches!(error, Error::Encoding(m) if m.contains("max_confirmed_in_flight")));
    }
}

#[tokio::test]
async fn admission_default_unconfirmed_limit_is_32_without_waiters() {
    let (mut server, _tx, mut started) = fixture_with_config(
        "admission",
        ServerConfig {
            discovery_policy: DiscoveryPolicy::unlimited(),
            request_admission_policy: RequestAdmissionPolicy {
                max_unconfirmed_in_flight_per_peer: 32,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .await;
    // Hold the real database seam, rather than expecting every Who-Is to reach
    // its send: a periodic writer can queue between readers of this fair RwLock.
    let db = Arc::clone(&server.db);
    let held_db = db.write().await;
    for _ in 0..32 {
        dispatch(&server, who_is(), None, None).await;
    }
    tokio::task::yield_now().await;
    dispatch(&server, who_is(), None, None).await;
    let c = server.request_admission_counters();
    assert_eq!(c.unconfirmed_active, 32);
    assert_eq!(c.unconfirmed_admitted_total, 32);
    assert_eq!(c.unconfirmed_overloaded_total, 1);
    assert!(started.try_recv().is_err());
    drop(held_db);
    server.stop().await.unwrap();
    assert_eq!(server.request_admission_counters().unconfirmed_active, 0);
}

#[tokio::test]
async fn admission_every_class_releases_on_panic_and_before_first_poll_cancellation() {
    for class in [Class::Confirmed, Class::Unconfirmed, Class::Abort] {
        let owner = crate::server::request_tasks::RequestTasks::default();
        owner
            .try_spawn(
                class,
                crate::server::request_peer::canonical_requester(&[1], None),
                || async { panic!("injected class panic") },
            )
            .unwrap();
        assert!(owner.join_next().await.unwrap().unwrap_err().is_panic());
        owner
            .try_spawn(
                class,
                crate::server::request_peer::canonical_requester(&[1], None),
                std::future::pending,
            )
            .unwrap();
        owner.close();
        assert!(owner.join_next().await.unwrap().unwrap_err().is_cancelled());
        let c = owner.counters();
        assert_eq!(
            c.confirmed_active + c.unconfirmed_active + c.abort_active,
            0
        );
    }
}

#[tokio::test]
async fn admission_abort_rechecks_dcc_when_first_polled() {
    let (mut server, _tx, mut started) = small_fixture().await;
    dispatch(&server, request(1), None, None).await;
    observed(&mut started).await;
    dispatch(&server, request(2), None, None).await;
    server.comm_state.store(1, Ordering::Release);
    tokio::time::timeout(Duration::from_secs(2), async {
        while server.request_admission_counters().abort_active != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(started.try_recv().is_err());
    assert_eq!(server.request_admission_counters().abort_admitted_total, 1);
    server.stop().await.unwrap();
}
