use super::*;
use crate::server::request_peer::canonical_requester;
use crate::server::request_tasks::RequestTasks;

#[tokio::test]
async fn peer_admission_default_caps_before_handler_spawn() {
    let (mut server, _tx, mut started) = fixture_with_config(
        "peer admission",
        ServerConfig {
            discovery_policy: DiscoveryPolicy::unlimited(),
            ..Default::default()
        },
    )
    .await;
    let db = Arc::clone(&server.db);
    let held = db.write().await;
    for id in 0..17 {
        dispatch(&server, request(id), None, None).await;
    }
    for _ in 0..9 {
        dispatch(&server, who_is(), None, None).await;
    }
    let c = server.request_admission_counters();
    assert_eq!(c.confirmed_active, 16);
    assert_eq!(c.unconfirmed_active, 8);
    assert_eq!(c.confirmed_overloaded_total, 1);
    assert_eq!(c.unconfirmed_overloaded_total, 1);
    assert_eq!(c.confirmed_peer_overloaded_total, 1);
    assert_eq!(c.unconfirmed_peer_overloaded_total, 1);
    assert_eq!(
        c.confirmed_global_overloaded_total + c.unconfirmed_global_overloaded_total,
        0
    );
    observed(&mut started).await; // Only the rejected request's Abort can send.
    assert!(started.try_recv().is_err());
    drop(held);
    server.stop().await.unwrap();
    assert_eq!(server.request_tasks.peer_entries(), [0; 3]);
}

fn route(network: u16, mac: &[u8]) -> NpduAddress {
    NpduAddress {
        network,
        mac_address: MacAddr::from_slice(mac),
    }
}

#[tokio::test]
async fn peer_admission_logical_identity_and_duplicate_fallback_matrix() {
    use crate::server::confirmed_request_tracker::{
        ConfirmedRequestAdmission, ConfirmedRequestTracker,
    };
    for source in [
        None,
        Some(route(0, b"origin")),
        Some(route(65535, b"origin")),
        Some(route(1, b"")),
        Some(route(65534, b"")),
        Some(route(1, b"origin")),
        Some(route(65534, b"origin")),
    ] {
        let valid = source
            .as_ref()
            .is_some_and(|s| s.network != 0 && s.network != 65535 && !s.mac_address.is_empty());
        let a = canonical_requester(b"router-a", source.as_ref());
        let b = canonical_requester(b"router-b", source.as_ref());
        assert_eq!(a == b, valid);
        assert_eq!(a == canonical_requester(b"router-a", None), !valid);
        let tracker = Arc::new(ConfirmedRequestTracker::default());
        let Apdu::ConfirmedRequest(req) = request(1) else {
            unreachable!()
        };
        let ConfirmedRequestAdmission::New(pending) =
            tracker.begin(b"router-a", source.as_ref(), req.clone())
        else {
            panic!("new")
        };
        assert!(matches!(
            tracker.begin(b"router-a", source.as_ref(), req.clone()),
            ConfirmedRequestAdmission::Duplicate
        ));
        assert_eq!(
            matches!(
                tracker.begin(b"router-b", source.as_ref(), req),
                ConfirmedRequestAdmission::Duplicate
            ),
            valid
        );
        let owner = RequestTasks::new(RequestAdmissionPolicy {
            max_confirmed_in_flight_per_peer: 1,
            ..Default::default()
        })
        .unwrap();
        owner
            .try_spawn(Class::Confirmed, a, std::future::pending)
            .unwrap();
        let result = owner.try_spawn(Class::Confirmed, b, std::future::pending);
        assert_eq!(
            result,
            if valid {
                Err(Rejection::Overloaded)
            } else {
                Ok(())
            }
        );
        owner.close();
        while !owner.is_empty() {
            owner.join_next().await;
        }
        assert_eq!(owner.peer_entries(), [0; 3]);
        drop(pending);
    }
}

#[tokio::test]
async fn peer_admission_independent_classes_origins_and_global_first() {
    let owner = RequestTasks::new(RequestAdmissionPolicy {
        max_confirmed_in_flight: 2,
        max_unconfirmed_in_flight: 2,
        max_confirmed_in_flight_per_peer: 1,
        max_unconfirmed_in_flight_per_peer: 1,
    })
    .unwrap();
    for class in [Class::Confirmed, Class::Unconfirmed] {
        let peer = canonical_requester(b"router-a", Some(&route(7, b"a")));
        owner
            .try_spawn(class, peer.clone(), std::future::pending)
            .unwrap();
        assert_eq!(
            owner.try_spawn(
                class,
                canonical_requester(b"router-b", Some(&route(7, b"a"))),
                || async { panic!("denied ran") }
            ),
            Err(Rejection::Overloaded)
        );
        owner
            .try_spawn(
                class,
                canonical_requester(b"router-a", Some(&route(7, b"b"))),
                std::future::pending,
            )
            .unwrap();
        assert_eq!(
            owner.try_spawn(class, peer, || async { panic!("denied ran") }),
            Err(Rejection::Overloaded)
        );
    }
    let c = owner.counters();
    assert_eq!((c.confirmed_active, c.unconfirmed_active), (2, 2));
    assert_eq!(
        (c.confirmed_admitted_total, c.unconfirmed_admitted_total),
        (2, 2)
    );
    assert_eq!(
        (
            c.confirmed_global_overloaded_total,
            c.confirmed_peer_overloaded_total
        ),
        (1, 1)
    );
    assert_eq!(
        (
            c.unconfirmed_global_overloaded_total,
            c.unconfirmed_peer_overloaded_total
        ),
        (1, 1)
    );
    assert_eq!(
        (c.confirmed_overloaded_total, c.unconfirmed_overloaded_total),
        (2, 2)
    );
    assert_eq!(owner.peer_entries(), [2, 2, 0]);
    owner.close();
    while !owner.is_empty() {
        owner.join_next().await;
    }
    assert_eq!(owner.peer_entries(), [0; 3]);
}

#[tokio::test]
async fn peer_admission_guard_cleanup_normal_panic_never_polled_and_unique_stream() {
    for class in [Class::Confirmed, Class::Unconfirmed, Class::Abort] {
        let owner = RequestTasks::default();
        let peer = canonical_requester(b"peer", None);
        owner
            .try_spawn(class, peer.clone(), || async { panic!("injected") })
            .unwrap();
        assert!(owner.join_next().await.unwrap().unwrap_err().is_panic());
        assert_eq!(owner.peer_entries(), [0; 3]);
        for id in 0u32..2048 {
            owner
                .try_spawn(
                    class,
                    canonical_requester(&id.to_be_bytes(), None),
                    || async {},
                )
                .unwrap();
            owner.join_next().await.unwrap().unwrap();
            assert_eq!(owner.peer_entries(), [0; 3]);
        }
        owner
            .try_spawn(class, peer.clone(), std::future::pending)
            .unwrap();
        owner.close(); // current-thread runtime: future has never been polled
        assert_eq!(
            owner.try_spawn(class, peer, || async { panic!("closed") }),
            Err(Rejection::Closed)
        );
        assert!(owner.join_next().await.unwrap().unwrap_err().is_cancelled());
        assert_eq!(owner.peer_entries(), [0; 3]);
        let c = owner.counters();
        assert_eq!(
            c.confirmed_overloaded_total
                + c.unconfirmed_overloaded_total
                + c.confirmed_fallback_dropped_total,
            0
        );
    }
}

#[tokio::test]
async fn peer_admission_duplicates_denied_retry_and_shared_eight_abort_workers() {
    let (mut server, _tx, mut started) = fixture_with_config(
        "peer abort",
        ServerConfig {
            request_admission_policy: RequestAdmissionPolicy {
                max_confirmed_in_flight_per_peer: 1,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .await;
    dispatch(&server, request(1), None, None).await;
    observed(&mut started).await;
    dispatch(&server, request(1), None, None).await;
    assert_eq!(
        server
            .request_admission_counters()
            .confirmed_overloaded_total,
        0
    );
    for id in 2..=9 {
        dispatch(&server, request(id), None, None).await;
        observed(&mut started).await;
    }
    dispatch(&server, request(10), None, None).await;
    let c = server.request_admission_counters();
    assert_eq!((c.confirmed_active, c.abort_active), (1, 8));
    assert_eq!(
        (
            c.confirmed_peer_overloaded_total,
            c.confirmed_global_overloaded_total
        ),
        (9, 0)
    );
    assert_eq!(
        (c.abort_admitted_total, c.confirmed_fallback_dropped_total),
        (8, 1)
    );
    assert_eq!(server.request_tasks.peer_entries(), [1, 0, 0]);
    assert!(started.try_recv().is_err());
    server.network.transport().release.notify_waiters();
    wait_reaped(&server).await;
    assert_eq!(server.request_tasks.peer_entries(), [0; 3]);
    dispatch(&server, request(10), None, None).await;
    observed(&mut started).await;
    dispatch(&server, request(1), None, None).await; // completed duplicate at peer cap
    assert_eq!(
        server.request_admission_counters().confirmed_admitted_total,
        2
    );
    assert_eq!(
        server
            .request_admission_counters()
            .confirmed_overloaded_total,
        9
    );
    server.stop().await.unwrap();
    assert_eq!(server.request_tasks.peer_entries(), [0; 3]);
}

#[tokio::test]
async fn peer_admission_direct_routed_reply_abort_fields_unchanged() {
    for source in [None, Some(route(42, &[7]))] {
        for reply_channel in [false, true] {
            let (mut server, _tx, mut started) = fixture_with_config(
                "peer reply",
                ServerConfig {
                    request_admission_policy: RequestAdmissionPolicy {
                        max_confirmed_in_flight_per_peer: 1,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .await;
            dispatch(&server, request(1), source.clone(), None).await;
            observed(&mut started).await;
            let abort = if reply_channel {
                let (tx, rx) = oneshot::channel();
                dispatch(&server, request(2), source.clone(), Some(tx)).await;
                let wire = tokio::time::timeout(Duration::from_secs(2), rx)
                    .await
                    .unwrap()
                    .unwrap();
                let npdu = decode_npdu(wire).unwrap();
                assert_eq!(npdu.destination, source);
                assert!(!npdu.expecting_reply);
                apdu::decode_apdu(npdu.payload).unwrap()
            } else {
                dispatch(&server, request(2), source.clone(), None).await;
                observed(&mut started).await;
                assert_eq!(
                    server
                        .network
                        .transport()
                        .routes
                        .lock()
                        .unwrap()
                        .last()
                        .unwrap(),
                    &(source.clone(), MacAddr::from_slice(&[1]))
                );
                server
                    .network
                    .transport()
                    .frames
                    .lock()
                    .unwrap()
                    .last()
                    .unwrap()
                    .clone()
            };
            assert!(
                matches!(abort, Apdu::Abort(a) if a.sent_by_server && a.invoke_id == 2 && a.abort_reason == AbortReason::OUT_OF_RESOURCES)
            );
            assert_eq!(
                server
                    .request_admission_counters()
                    .confirmed_peer_overloaded_total,
                1
            );
            server.stop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn peer_admission_positive_validation_generic_bip_and_tiny_global() {
    for confirmed in [true, false] {
        for bad in [0, Semaphore::MAX_PERMITS + 1, usize::MAX] {
            let mut policy = RequestAdmissionPolicy::default();
            let name = if confirmed {
                policy.max_confirmed_in_flight_per_peer = bad;
                "max_confirmed_in_flight_per_peer"
            } else {
                policy.max_unconfirmed_in_flight_per_peer = bad;
                "max_unconfirmed_in_flight_per_peer"
            };
            assert!(policy.validate().is_err());
            let started = Arc::new(AtomicBool::new(false));
            let error = BACnetServer::generic_builder()
                .transport(NeverStart(Arc::clone(&started)))
                .request_admission_policy(policy)
                .build()
                .await
                .err()
                .unwrap();
            assert!(matches!(error, Error::Encoding(m) if m.contains(name)));
            assert!(!started.load(Ordering::Acquire));
            let error = BACnetServer::bip_builder()
                .interface(Ipv4Addr::LOCALHOST)
                .port(0)
                .request_admission_policy(policy)
                .build()
                .await
                .err()
                .unwrap();
            assert!(matches!(error, Error::Encoding(m) if m.contains(name)));
        }
    }
    let owner = RequestTasks::new(RequestAdmissionPolicy {
        max_confirmed_in_flight: 1,
        max_unconfirmed_in_flight: 1,
        ..Default::default()
    })
    .unwrap();
    for class in [Class::Confirmed, Class::Unconfirmed] {
        let peer = canonical_requester(b"peer", None);
        owner
            .try_spawn(class, peer.clone(), std::future::pending)
            .unwrap();
        assert_eq!(
            owner.try_spawn(class, peer, std::future::pending),
            Err(Rejection::Overloaded)
        );
    }
    let c = owner.counters();
    assert_eq!(
        (
            c.confirmed_global_overloaded_total,
            c.unconfirmed_global_overloaded_total
        ),
        (1, 1)
    );
    assert_eq!(
        c.confirmed_peer_overloaded_total + c.unconfirmed_peer_overloaded_total,
        0
    );
    owner.close();
    while !owner.is_empty() {
        owner.join_next().await;
    }
}
