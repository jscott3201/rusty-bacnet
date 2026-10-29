use super::*;
use crate::server::request_admission::{Class, Rejection};
use crate::server::test_transport::{SendMode, StartMode};

#[path = "peer_admission_tests.rs"]
mod peer_admission_tests;

#[path = "dcc_outcome_admission_tests.rs"]
mod dcc_outcome_admission_tests;
#[path = "recovery_admission_tests.rs"]
mod recovery_admission_tests;

// Exercise sealed admission before final transport retirement; after successful
// stop there is deliberately no network owner left for direct private dispatch.
async fn begin_stop(server: &mut BACnetServer<TestTransport>) {
    use std::future::Future;
    let stop = server.stop();
    tokio::pin!(stop);
    std::future::poll_fn(|cx| {
        assert!(stop.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
}

async fn small_fixture() -> (
    BACnetServer<TestTransport>,
    mpsc::Sender<ReceivedNpdu>,
    mpsc::UnboundedReceiver<oneshot::Receiver<()>>,
) {
    fixture_with_config(
        "admission",
        ServerConfig {
            device: bacnet_objects::DeviceIdentity::default().with_segmentation(Segmentation::BOTH),
            request_admission_policy: RequestAdmissionPolicy {
                max_confirmed_in_flight: 1,
                confirmed_recovery_reserve: 0,
                max_unconfirmed_in_flight: 1,
                ..Default::default()
            },
            discovery_policy: DiscoveryPolicy::unlimited(),
            ..ServerConfig::default()
        },
    )
    .await
}

async fn dispatch(
    server: &BACnetServer<TestTransport>,
    apdu: Apdu,
    source: Option<NpduAddress>,
    reply_tx: Option<oneshot::Sender<Bytes>>,
) {
    BACnetServer::dispatch(
        &server.test_dispatch_context(),
        &[1],
        apdu,
        bacnet_network::layer::ReceivedApdu {
            router_response: None,
            direct_response: None,
            apdu: Bytes::new(),
            source_mac: MacAddr::from_slice(&[1]),
            ingress_network: None,
            source_network: source,
            link_layer_group: false,
            is_group: false,
            global_broadcast: false,
            data_attributes: Vec::new(),
            provenance: bacnet_transport::port::TransportProvenance::unverified(),
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
                confirmed_recovery_reserve: 0,
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
        let frames = held_sends(&server).frames.lock().unwrap();
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
                sent_by_server: true,
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
            Ok(match kind {
                0 => CovAckResult::Ack,
                1 => CovAckResult::Error(RemoteError::Error {
                    class: ErrorClass::SERVICES,
                    code: ErrorCode::OTHER,
                }),
                2 => CovAckResult::Error(RemoteError::Reject {
                    reason: RejectReason::OTHER
                }),
                _ => CovAckResult::Error(RemoteError::Abort {
                    reason: AbortReason::OTHER
                }),
            })
        );
        drop(operation);
    }
    assert_eq!(server.request_admission_counters(), counters);
    assert!(started.try_recv().is_err());
    assert_eq!(held_sends(&server).frames.lock().unwrap().len(), 10);
    held_sends(&server).release.notify_waiters();
    original.await.unwrap();
    unconfirmed.await.unwrap();
    wait_reaped(&server).await;
    // Denied work never starts after release. All ten prior frames are final.
    assert_eq!(held_sends(&server).frames.lock().unwrap().len(), 10);
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
async fn admission_pending_duplicate_precedes_capacity_but_issued_reuse_overloads() {
    let (mut server, _tx, mut started) = small_fixture().await;
    let db = server.db.clone();
    let held = db.write().await;
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
    assert!(started.try_recv().is_err());
    drop(held);
    let original = observed(&mut started).await;
    dispatch(&server, request(1), None, None).await;
    let overloaded = observed(&mut started).await;
    assert_eq!(
        server.request_admission_counters().confirmed_admitted_total,
        1
    );
    assert_eq!(
        server
            .request_admission_counters()
            .confirmed_overloaded_total,
        1
    );
    assert_eq!(server.request_admission_counters().abort_admitted_total, 1);
    held_sends(&server).release.notify_waiters();
    original.await.unwrap();
    overloaded.await.unwrap();
    wait_reaped(&server).await;
    dispatch(&server, request(1), None, None).await;
    observed(&mut started).await;
    assert_eq!(
        server.request_admission_counters().confirmed_admitted_total,
        2
    );
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
    assert_eq!(held_sends(&server).frames.lock().unwrap().len(), 1);
    server.stop().await.unwrap();
    assert_eq!(server.request_admission_counters().abort_active, 0);
}

#[tokio::test]
async fn admission_stop_seals_classes_without_overload_and_joins_abort_workers() {
    let (mut server, _tx, mut started) = small_fixture().await;
    dispatch(&server, request(1), None, None).await;
    let original = observed(&mut started).await;
    dispatch(&server, request(2), None, None).await;
    let abort = observed(&mut started).await;
    begin_stop(&mut server).await;
    original.await.unwrap();
    abort.await.unwrap();
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
    server.stop().await.unwrap();
}

#[tokio::test]
async fn admission_panic_releases_real_handler_and_abort_and_allows_retry() {
    let (mut server, tx, mut started) = small_fixture().await;
    for id in [1, 1] {
        inject(&tx, request(id)).await;
        let released = observed(&mut started).await;
        held_sends(&server)
            .panic_next
            .store(true, Ordering::Release);
        held_sends(&server).release.notify_one();
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
        confirmed_recovery_reserve: 0,
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
            held_sends(&server).routes.lock().unwrap().last().unwrap(),
            &(route, MacAddr::from_slice(&[1]))
        );
        held_sends(&server)
            .panic_next
            .store(panic, Ordering::Release);
        held_sends(&server)
            .fail_next
            .store(!panic, Ordering::Release);
        held_sends(&server).release.notify_one();
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

/// Records that startup was reached, then panics; its sends are unreachable.
fn never_start(started: &Arc<AtomicBool>) -> TestTransport {
    let started = Arc::clone(started);
    TestTransport::builder()
        .on_start(move || started.store(true, Ordering::Release))
        .start(StartMode::Panic(
            "invalid admission must not start transport",
        ))
        .unicast(SendMode::Panic("unreachable send"))
        .broadcast(SendMode::Panic("unreachable send"))
        .build()
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
            never_start(&started),
        )
        .await
        .err()
        .unwrap();
        assert!(matches!(error, Error::Encoding(m) if m.contains("max_confirmed_in_flight")));
        let error = BACnetServer::generic_builder()
            .transport(never_start(&started))
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
