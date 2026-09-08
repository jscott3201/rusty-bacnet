use super::*;
use crate::server::request_peer::canonical_requester;
use crate::server::request_tasks::RequestTasks;
use bacnet_services::device_mgmt::DeviceCommunicationControlRequest;
use bacnet_types::enums::EnableDisable;

fn peer(id: u8) -> crate::server::request_peer::CanonicalRequester {
    canonical_requester(&[id], None)
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
async fn recovery_peer_total_is_inclusive_and_protected_peer_is_additional() {
    for recovery_first in [false, true] {
        let owner = RequestTasks::default();
        if recovery_first {
            owner
                .try_spawn(Class::Recovery, peer(1), std::future::pending)
                .unwrap();
        }
        for _ in 0..if recovery_first { 15 } else { 16 } {
            owner
                .try_spawn(Class::Confirmed, peer(1), std::future::pending)
                .unwrap();
        }
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
            (18, 3, 0)
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
