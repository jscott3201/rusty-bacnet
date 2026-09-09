use super::*;
use crate::server::request_peer::canonical_requester;
use crate::server::request_tasks::RequestTasks;
use bacnet_services::device_mgmt::DeviceCommunicationControlRequest;
use bacnet_types::enums::EnableDisable;

fn peer(id: u8) -> crate::server::request_peer::CanonicalRequester {
    canonical_requester(&[id], None)
}

#[test]
fn recovery_small_quota_table_release_refill_and_global_first() {
    use crate::server::request_admission::Admission;

    for global in 1..=6 {
        for reserve in 0..global {
            for normal_peer in 1..=7 {
                for recovery_peer in 1..=7 {
                    for recovery_first in [false, true] {
                        let admission = Admission::new(RequestAdmissionPolicy {
                            max_confirmed_in_flight: global,
                            confirmed_recovery_reserve: reserve,
                            max_confirmed_in_flight_per_peer: normal_peer,
                            max_recovery_in_flight_per_peer: recovery_peer,
                            ..Default::default()
                        })
                        .unwrap();
                        let ordinary = normal_peer.min(global - reserve);
                        let protected = recovery_peer.min(reserve);
                        let order = if recovery_first {
                            [(Class::Recovery, protected), (Class::Confirmed, ordinary)]
                        } else {
                            [(Class::Confirmed, ordinary), (Class::Recovery, protected)]
                        };
                        let mut held = Vec::new();
                        let mut admitted = 0;
                        let mut recovery_admitted = 0;
                        for (class, count) in order {
                            for _ in 0..count {
                                held.push((
                                    class,
                                    Some(admission.try_enter(class, peer(1), false).unwrap()),
                                ));
                                admitted += 1;
                                recovery_admitted += usize::from(matches!(class, Class::Recovery));
                            }
                        }
                        let c = admission.snapshot();
                        assert_eq!(
                            (c.confirmed_active, c.recovery_active),
                            (ordinary + protected, protected)
                        );
                        // Both partitions stay full at this peer while each slot is
                        // independently released/refilled, including the opposite order.
                        for (class, guard) in &mut held {
                            let replacement = admission.try_enter(*class, peer(1), false);
                            assert!(matches!(replacement, Err(Rejection::Overloaded)));
                            // Drop exactly one guard without releasing the other class.
                            drop(guard.take());
                            *guard = Some(admission.try_enter(*class, peer(1), false).unwrap());
                            admitted += 1;
                            recovery_admitted += usize::from(matches!(class, Class::Recovery));
                        }
                        let before = admission.snapshot();
                        for class in [Class::Confirmed, Class::Recovery] {
                            assert!(matches!(
                                admission.try_enter(class, peer(1), false),
                                Err(Rejection::Overloaded)
                            ));
                        }
                        let after = admission.snapshot();
                        let global_denials = usize::from(ordinary == global - reserve)
                            + usize::from(if reserve == 0 {
                                ordinary == global
                            } else {
                                protected == reserve
                            });
                        assert_eq!(
                            after.confirmed_global_overloaded_total
                                - before.confirmed_global_overloaded_total,
                            global_denials as u64
                        );
                        assert_eq!(
                            after.confirmed_peer_overloaded_total
                                - before.confirmed_peer_overloaded_total,
                            (2 - global_denials) as u64
                        );
                        assert_eq!(after.confirmed_admitted_total, admitted);
                        assert_eq!(after.recovery_admitted_total, recovery_admitted as u64);
                        assert_eq!(
                            after.confirmed_overloaded_total,
                            after.confirmed_global_overloaded_total
                                + after.confirmed_peer_overloaded_total
                        );
                        // Peer-denied temporary permits must be available to other peers.
                        for (class, count) in [
                            (Class::Confirmed, global - reserve - ordinary),
                            (Class::Recovery, reserve - protected),
                        ] {
                            for id in 0..count {
                                held.push((
                                    class,
                                    Some(
                                        admission
                                            .try_enter(class, peer(2 + id as u8), false)
                                            .unwrap(),
                                    ),
                                ));
                            }
                        }
                        assert_eq!(admission.snapshot().confirmed_active, global);
                        for class in [Class::Confirmed, Class::Recovery] {
                            assert!(matches!(
                                admission.try_enter(class, peer(99), false),
                                Err(Rejection::Overloaded)
                            ));
                        }
                        drop(held);
                        assert_eq!(admission.peer_entries(), [0; 3]);
                        assert_eq!(admission.snapshot().confirmed_active, 0);
                        assert_eq!(admission.snapshot().recovery_active, 0);
                        // R=0 routes Recovery through the ordinary peer/global caps.
                        let guard = admission
                            .try_enter(Class::Recovery, peer(1), false)
                            .unwrap();
                        assert_eq!(
                            admission.snapshot().recovery_active,
                            usize::from(reserve != 0)
                        );
                        drop(guard);
                        assert_eq!(admission.peer_entries(), [0; 3]);
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn recovery_wire_same_peer_enable_with_sixteen_ordinary_held() {
    let (mut server, tx, mut started) = fixture().await;
    for id in 0..16 {
        inject(&tx, request(id)).await;
        observed(&mut started).await; // Transport barrier: ordinary handler stays live.
    }
    assert_eq!(server.request_admission_counters().confirmed_active, 16);
    server.comm_state.store(1, Ordering::Release);
    inject(&tx, enable(16, None)).await;
    observed(&mut started).await;
    assert!(
        matches!(server.network.transport().frames.lock().unwrap().last(), Some(Apdu::SimpleAck(a)) if a.invoke_id == 16)
    );
    assert_eq!(server.comm_state.load(Ordering::Acquire), 0);
    let c = server.request_admission_counters();
    assert_eq!((c.confirmed_active, c.recovery_active), (17, 1));
    assert_eq!(
        (c.confirmed_admitted_total, c.recovery_admitted_total),
        (17, 1)
    );
    assert_eq!(c.confirmed_overloaded_total, 0);
    server.network.transport().release.notify_waiters();
    wait_reaped(&server).await;
    assert_eq!(server.request_tasks.peer_entries(), [0; 3]);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn recovery_segmented_enable_charged_once_after_reassembly() {
    let (mut server, tx, mut started) = fixture().await;
    server.comm_state.store(1, Ordering::Release);
    let Apdu::ConfirmedRequest(mut first) = enable(42, None) else {
        unreachable!()
    };
    let mut last = first.clone();
    first.segmented = true;
    first.more_follows = true;
    first.sequence_number = Some(0);
    first.proposed_window_size = Some(1);
    first.service_request = first.service_request.slice(..1);
    last.segmented = true;
    last.sequence_number = Some(1);
    last.proposed_window_size = Some(1);
    last.service_request = last.service_request.slice(1..);
    inject(&tx, Apdu::ConfirmedRequest(first)).await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while server.network.transport().frames.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        server.request_admission_counters().confirmed_admitted_total,
        0
    );
    assert_eq!(server.comm_state.load(Ordering::Acquire), 1);
    inject(&tx, Apdu::ConfirmedRequest(last)).await;
    observed(&mut started).await;
    let c = server.request_admission_counters();
    assert_eq!(
        (
            c.confirmed_active,
            c.recovery_active,
            c.confirmed_admitted_total,
            c.recovery_admitted_total
        ),
        (1, 1, 1, 1)
    );
    assert_eq!(server.comm_state.load(Ordering::Acquire), 0);
    server.stop().await.unwrap();
}

fn source(id: u8) -> Option<NpduAddress> {
    Some(NpduAddress {
        network: 7,
        mac_address: MacAddr::from_slice(&[id]),
    })
}

fn enable(id: u8, password: Option<&str>) -> Apdu {
    let Apdu::ConfirmedRequest(mut req) = request(id) else {
        unreachable!()
    };
    req.service_choice = ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL;
    let mut data = BytesMut::new();
    DeviceCommunicationControlRequest {
        time_duration: None,
        enable_disable: EnableDisable::ENABLE,
        password: password.map(str::to_owned),
    }
    .encode(&mut data)
    .unwrap();
    req.service_request = data.freeze();
    Apdu::ConfirmedRequest(req)
}

async fn drain(owner: &RequestTasks) {
    owner.close();
    while !owner.is_empty() {
        owner.join_next().await;
    }
    assert_eq!(owner.peer_entries(), [0; 3]);
    assert_eq!(
        (
            owner.counters().confirmed_active,
            owner.counters().recovery_active
        ),
        (0, 0)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovery_spawn_close_race_reclaims_both_peer_maps() {
    tokio::time::timeout(Duration::from_secs(5), async {
        for _ in 0..32 {
            let owner = Arc::new(RequestTasks::default());
            let barrier = Arc::new(tokio::sync::Barrier::new(3));
            let mut writers = Vec::new();
            for class in [Class::Confirmed, Class::Recovery] {
                let owner = Arc::clone(&owner);
                let barrier = Arc::clone(&barrier);
                writers.push(tokio::spawn(async move {
                    barrier.wait().await;
                    owner.try_spawn(class, peer(1), std::future::pending)
                }));
            }
            barrier.wait().await;
            owner.close();
            for writer in writers {
                assert!(matches!(
                    writer.await.unwrap(),
                    Ok(()) | Err(Rejection::Closed)
                ));
            }
            drain(&owner).await;
            let c = owner.counters();
            assert_eq!(
                c.confirmed_admitted_total + c.confirmed_shutdown_rejected_total,
                2
            );
            assert_eq!(c.confirmed_overloaded_total, 0);
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn recovery_default_ordinary_partition_is_sixty_not_sixty_four() {
    let owner = RequestTasks::default();
    for id in 0u8..60 {
        owner
            .try_spawn(
                Class::Confirmed,
                canonical_requester(&[id], None),
                std::future::pending,
            )
            .unwrap();
    }
    assert_eq!(
        owner.try_spawn(
            Class::Confirmed,
            canonical_requester(&[60], None),
            std::future::pending
        ),
        Err(Rejection::Overloaded)
    );
    assert_eq!(owner.counters().confirmed_active, 60);
    owner.close();
    while !owner.is_empty() {
        owner.join_next().await;
    }
    assert_eq!(owner.peer_entries(), [0; 3]);
}

#[tokio::test]
async fn recovery_strict_partitions_no_lending_and_aggregate_counters() {
    let owner = RequestTasks::default();
    for id in 0..4 {
        owner
            .try_spawn(Class::Recovery, peer(id), std::future::pending)
            .unwrap();
    }
    assert_eq!(
        owner.try_spawn(Class::Recovery, peer(4), std::future::pending),
        Err(Rejection::Overloaded)
    );
    for id in 0..60 {
        owner
            .try_spawn(Class::Confirmed, peer(id), std::future::pending)
            .unwrap();
    }
    assert_eq!(
        owner.try_spawn(Class::Confirmed, peer(60), std::future::pending),
        Err(Rejection::Overloaded)
    );
    let c = owner.counters();
    assert_eq!((c.confirmed_active, c.recovery_active), (64, 4));
    assert_eq!(
        (c.confirmed_admitted_total, c.recovery_admitted_total),
        (64, 4)
    );
    assert_eq!(
        (c.confirmed_overloaded_total, c.recovery_overloaded_total),
        (2, 1)
    );
    assert_eq!(
        (
            c.confirmed_global_overloaded_total,
            c.confirmed_peer_overloaded_total
        ),
        (2, 0)
    );
    drain(&owner).await;
}

#[tokio::test]
async fn recovery_peer_quotas_are_independent_in_both_arrival_orders() {
    fn hold(owner: &RequestTasks, class: Class, wait: oneshot::Receiver<()>) {
        owner
            .try_spawn(class, peer(1), || async move {
                wait.await.unwrap();
            })
            .unwrap();
    }
    for recovery_first in [false, true] {
        let owner = RequestTasks::default();
        let (recovery_release, recovery_wait) = oneshot::channel::<()>();
        let mut recovery_wait = Some(recovery_wait);
        if recovery_first {
            hold(&owner, Class::Recovery, recovery_wait.take().unwrap());
        }
        let (ordinary_release, ordinary_wait) = oneshot::channel::<()>();
        hold(&owner, Class::Confirmed, ordinary_wait);
        for _ in 0..15 {
            owner
                .try_spawn(Class::Confirmed, peer(1), std::future::pending)
                .unwrap();
        }
        if !recovery_first {
            hold(&owner, Class::Recovery, recovery_wait.take().unwrap());
        }
        ordinary_release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), owner.join_next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let c = owner.counters();
        assert_eq!((c.confirmed_active, c.recovery_active), (16, 1));
        owner
            .try_spawn(Class::Confirmed, peer(1), std::future::pending)
            .unwrap();
        recovery_release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), owner.join_next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let c = owner.counters();
        assert_eq!((c.confirmed_active, c.recovery_active), (16, 0));
        owner
            .try_spawn(Class::Recovery, peer(1), std::future::pending)
            .unwrap();
        for class in [Class::Recovery, Class::Confirmed] {
            assert_eq!(
                owner.try_spawn(class, peer(1), std::future::pending),
                Err(Rejection::Overloaded)
            );
        }
        owner
            .try_spawn(Class::Recovery, peer(2), std::future::pending)
            .unwrap();
        assert_eq!(
            owner.try_spawn(Class::Recovery, peer(2), std::future::pending),
            Err(Rejection::Overloaded)
        );
        owner
            .try_spawn(Class::Recovery, peer(3), std::future::pending)
            .unwrap();
        let c = owner.counters();
        assert_eq!(
            (
                c.confirmed_active,
                c.confirmed_peer_overloaded_total,
                c.confirmed_global_overloaded_total
            ),
            (19, 3, 0)
        );
        assert_eq!(c.recovery_overloaded_total, 2);
        drain(&owner).await;
    }
}

#[tokio::test]
async fn recovery_zero_reserve_uses_ordinary_and_preserves_tiny_global() {
    let owner = RequestTasks::new(RequestAdmissionPolicy {
        max_confirmed_in_flight: 1,
        confirmed_recovery_reserve: 0,
        ..Default::default()
    })
    .unwrap();
    owner
        .try_spawn(Class::Recovery, peer(1), std::future::pending)
        .unwrap();
    assert_eq!(
        owner.try_spawn(Class::Confirmed, peer(2), std::future::pending),
        Err(Rejection::Overloaded)
    );
    assert_eq!(
        (
            owner.counters().confirmed_active,
            owner.counters().recovery_active
        ),
        (1, 0)
    );
    drain(&owner).await;
}

#[tokio::test]
async fn recovery_guards_cleanup_panic_cancel_never_polled_and_closed() {
    let owner = RequestTasks::default();
    owner
        .try_spawn(Class::Recovery, peer(1), || async { panic!("injected") })
        .unwrap();
    assert!(owner.join_next().await.unwrap().unwrap_err().is_panic());
    assert_eq!(owner.peer_entries(), [0; 3]);
    for _ in 0..32 {
        owner
            .try_spawn(Class::Recovery, peer(1), || async {})
            .unwrap();
        owner.join_next().await.unwrap().unwrap();
    }
    owner
        .try_spawn(Class::Recovery, peer(1), std::future::pending)
        .unwrap();
    drain(&owner).await;
    assert_eq!(
        owner.try_spawn(Class::Recovery, peer(1), || async { panic!("closed") }),
        Err(Rejection::Closed)
    );
    assert_eq!(owner.counters().confirmed_shutdown_rejected_total, 1);
    assert_eq!(owner.counters().confirmed_overloaded_total, 0);
}

#[tokio::test]
async fn recovery_invalid_reserve_before_direct_generic_and_bip_start() {
    for (global, reserve) in [(64, 64), (64, 65), (1, 4), (4, 4)] {
        let policy = RequestAdmissionPolicy {
            max_confirmed_in_flight: global,
            confirmed_recovery_reserve: reserve,
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
        assert!(matches!(error, Error::Encoding(m) if m.contains("confirmed_recovery_reserve")));
        let error = BACnetServer::generic_builder()
            .transport(NeverStart(Arc::clone(&started)))
            .request_admission_policy(policy)
            .build()
            .await
            .err()
            .unwrap();
        assert!(matches!(error, Error::Encoding(m) if m.contains("confirmed_recovery_reserve")));
        assert!(!started.load(Ordering::Acquire));
        assert!(BACnetServer::bip_builder()
            .interface(Ipv4Addr::LOCALHOST)
            .port(0)
            .request_admission_policy(policy)
            .build()
            .await
            .is_err());
    }
    for reserve in [0, 4] {
        for bad in [0, usize::MAX] {
            assert!(RequestAdmissionPolicy {
                confirmed_recovery_reserve: reserve,
                max_recovery_in_flight_per_peer: bad,
                ..Default::default()
            }
            .validate()
            .is_err());
        }
    }
}

#[tokio::test]
async fn recovery_wire_enable_restores_communications_at_ordinary_saturation() {
    let (mut server, tx, mut started) = fixture().await;
    for id in 0..60 {
        dispatch(&server, request(id), source(id / 15), None).await;
        observed(&mut started).await;
    }
    dispatch(&server, request(60), source(4), None).await;
    observed(&mut started).await;
    assert!(
        matches!(server.network.transport().frames.lock().unwrap().last(), Some(Apdu::Abort(a)) if a.invoke_id == 60 && a.abort_reason == AbortReason::OUT_OF_RESOURCES)
    );
    server.comm_state.store(1, Ordering::Release);
    inject(&tx, enable(61, None)).await; // Full NPDU/APDU ingress, different logical peer.
    observed(&mut started).await;
    assert_eq!(server.comm_state.load(Ordering::Acquire), 0);
    assert!(
        matches!(server.network.transport().frames.lock().unwrap().last(), Some(Apdu::SimpleAck(a)) if a.invoke_id == 61)
    );
    assert_eq!(
        (
            server.request_admission_counters().confirmed_active,
            server.request_admission_counters().recovery_active
        ),
        (61, 1)
    );
    server.stop().await.unwrap();
    assert_eq!(server.request_tasks.peer_entries(), [0; 3]);
}

#[tokio::test]
async fn recovery_protected_wire_exhaustion_duplicate_retry_and_abort_fallback() {
    let (mut server, _tx, mut started) = fixture().await;
    for id in 0..4 {
        dispatch(&server, enable(id, None), source(id), None).await;
        observed(&mut started).await;
    }
    // Exact duplicates are quiet even when their partition and peer are full.
    for id in 0..4 {
        dispatch(&server, enable(id, None), source(id), None).await;
    }
    assert_eq!(
        server
            .request_admission_counters()
            .confirmed_overloaded_total,
        0
    );
    for id in 4..13 {
        dispatch(&server, enable(id, None), source(id), None).await;
        if id < 12 {
            observed(&mut started).await;
        }
    }
    let c = server.request_admission_counters();
    assert_eq!(
        (c.confirmed_active, c.recovery_active, c.abort_active),
        (4, 4, 8)
    );
    assert_eq!(
        (
            c.recovery_overloaded_total,
            c.confirmed_fallback_dropped_total
        ),
        (9, 1)
    );
    assert!(server.network.transport().frames.lock().unwrap()[4..].iter().all(|p| matches!(p, Apdu::Abort(a) if a.abort_reason == AbortReason::OUT_OF_RESOURCES && a.sent_by_server)));
    server.network.transport().release.notify_waiters();
    tokio::time::timeout(Duration::from_secs(2), async {
        while server.request_admission_counters().confirmed_active
            + server.request_admission_counters().abort_active
            != 0
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    dispatch(&server, enable(12, None), source(12), None).await;
    observed(&mut started).await;
    assert!(
        matches!(server.network.transport().frames.lock().unwrap().last(), Some(Apdu::SimpleAck(a)) if a.invoke_id == 12)
    );
    server.stop().await.unwrap();
}

#[tokio::test]
async fn recovery_classification_is_not_password_authorization() {
    let (mut server, _tx, mut started) = fixture_with_config(
        "recovery",
        ServerConfig {
            dcc_password: Some("required".into()),
            ..Default::default()
        },
    )
    .await;
    server.comm_state.store(1, Ordering::Release);
    for (id, password) in [(1, None), (2, Some("wrong"))] {
        dispatch(&server, enable(id, password), source(id), None).await;
        observed(&mut started).await;
        assert_eq!(server.comm_state.load(Ordering::Acquire), 1);
        assert!(
            matches!(server.network.transport().frames.lock().unwrap().last(), Some(Apdu::Error(e)) if e.error_code == ErrorCode::PASSWORD_FAILURE)
        );
    }
    assert_eq!(server.request_admission_counters().recovery_active, 2);
    dispatch(&server, enable(3, Some("required")), source(3), None).await;
    observed(&mut started).await;
    assert_eq!(server.comm_state.load(Ordering::Acquire), 0);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn recovery_noneligible_requests_cannot_borrow_and_password_capacity_precedes_handler() {
    let (mut server, _tx, mut started) = fixture_with_config(
        "recovery",
        ServerConfig {
            request_admission_policy: RequestAdmissionPolicy {
                max_confirmed_in_flight: 2,
                confirmed_recovery_reserve: 1,
                ..Default::default()
            },
            dcc_password: Some("required".into()),
            ..Default::default()
        },
    )
    .await;
    dispatch(&server, request(0), source(0), None).await;
    observed(&mut started).await;
    for (id, service, data) in [
        (
            1,
            ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
            &[0x19, 1][..],
        ),
        (
            2,
            ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
            &[0x19, 2][..],
        ),
        (
            3,
            ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
            &[0x19][..],
        ),
        (
            4,
            ConfirmedServiceChoice::REINITIALIZE_DEVICE,
            &[0x09, 0][..],
        ),
    ] {
        let Apdu::ConfirmedRequest(mut req) = request(id) else {
            unreachable!()
        };
        req.service_choice = service;
        req.service_request = Bytes::copy_from_slice(data);
        dispatch(&server, Apdu::ConfirmedRequest(req), source(id), None).await;
        observed(&mut started).await;
        assert!(
            matches!(server.network.transport().frames.lock().unwrap().last(), Some(Apdu::Abort(a)) if a.invoke_id == id)
        );
    }
    assert_eq!(
        server.request_admission_counters().recovery_admitted_total,
        0
    );
    dispatch(&server, enable(5, None), source(5), None).await;
    observed(&mut started).await;
    assert!(
        matches!(server.network.transport().frames.lock().unwrap().last(), Some(Apdu::Error(e)) if e.error_code == ErrorCode::PASSWORD_FAILURE)
    );
    dispatch(&server, enable(6, Some("wrong")), source(6), None).await;
    observed(&mut started).await;
    assert!(
        matches!(server.network.transport().frames.lock().unwrap().last(), Some(Apdu::Abort(a)) if a.invoke_id == 6)
    );
    assert_eq!(
        server
            .request_admission_counters()
            .recovery_overloaded_total,
        1
    );
    server.stop().await.unwrap();
}
