use super::*;
use crate::server::dcc_outcomes::trace_tests::Capture;

fn life_safety(id: u8) -> Apdu {
    let Apdu::ConfirmedRequest(mut req) = request(id) else {
        unreachable!()
    };
    let mut data = BytesMut::new();
    bacnet_services::life_safety::LifeSafetyOperationRequest {
        requesting_process_identifier: 7,
        requesting_source: "operator".into(),
        request: bacnet_types::enums::LifeSafetyOperation::SILENCE,
        object_identifier: None,
    }
    .encode(&mut data)
    .unwrap();
    req.service_choice = ConfirmedServiceChoice::LIFE_SAFETY_OPERATION;
    req.service_request = data.freeze();
    Apdu::ConfirmedRequest(req)
}

fn dcc(id: u8) -> Apdu {
    let Apdu::ConfirmedRequest(mut req) = request(id) else {
        unreachable!()
    };
    req.service_choice = ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL;
    req.service_request = Bytes::from_static(b"\x19\x00");
    Apdu::ConfirmedRequest(req)
}

#[tokio::test]
async fn dcc_outcomes_exclude_global_peer_and_abort_fallback_rejections() {
    for (global, peer) in [(1, 16), (4, 1)] {
        let capture = Capture::default();
        let _subscriber = tracing::subscriber::set_default(capture.clone());
        let (mut server, _, mut started) = fixture_with_config(
            "overload",
            ServerConfig {
                request_admission_policy: RequestAdmissionPolicy {
                    max_confirmed_in_flight: global,
                    max_confirmed_in_flight_per_peer: peer,
                    confirmed_recovery_reserve: 0,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await;
        dispatch(&server, dcc(1), None, None).await;
        let mut completions = vec![observed(&mut started).await];
        for id in 2..=9 {
            dispatch(&server, dcc(id), None, None).await;
            completions.push(observed(&mut started).await);
        }
        dispatch(&server, dcc(10), None, None).await;
        let counters = server.request_admission_counters();
        assert_eq!(
            counters.confirmed_global_overloaded_total,
            if global == 1 { 9 } else { 0 }
        );
        assert_eq!(
            counters.confirmed_peer_overloaded_total,
            if peer == 1 { 9 } else { 0 }
        );
        assert_eq!(counters.abort_admitted_total, 8);
        assert_eq!(counters.confirmed_fallback_dropped_total, 1);
        assert_eq!(server.dcc_outcome_counters().policy_denied_total, 1);
        assert_eq!(capture.0.lock().unwrap().len(), 1);
        server.stop().await.unwrap();
        for completion in completions {
            completion.await.unwrap();
        }
        assert_eq!(capture.0.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn dcc_outcomes_recovery_denied_duplicate_overload_and_shutdown() {
    // Current-thread runtime: this scoped default also covers spawned handlers.
    // No global subscriber is installed and no events escape the test.
    let capture = Capture::default();
    let _subscriber = tracing::subscriber::set_default(capture.clone());
    let (mut server, _, mut started) = fixture_with_config(
        "outcomes admission",
        ServerConfig {
            request_admission_policy: RequestAdmissionPolicy {
                max_confirmed_in_flight: 2,
                confirmed_recovery_reserve: 1,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .await;
    dispatch(&server, request(1), None, None).await;
    let ordinary = observed(&mut started).await;
    assert!(
        tracing::dispatcher::get_default(|d| d.is::<Capture>()),
        "lost scoped subscriber before dispatch"
    );
    dispatch(&server, dcc(2), None, None).await;
    dispatch(&server, dcc(2), None, None).await; // pending before its first poll
    assert_eq!(
        server.request_admission_counters().confirmed_admitted_total,
        2
    );
    let recovery = observed(&mut started).await;
    assert_eq!(server.dcc_outcome_counters().policy_denied_total, 1);
    assert_eq!(
        server.request_admission_counters().recovery_admitted_total,
        1
    );
    assert_eq!(capture.0.lock().unwrap().len(), 1);
    assert_eq!(capture.0.lock().unwrap()[0]["outcome"], "policy_denied");
    // Its response has now issued, although the transport Result is held.
    // Reuse is a new operation and the independent recovery task quota is full.
    dispatch(&server, dcc(2), None, None).await;
    let reused_abort = observed(&mut started).await;
    assert_eq!(
        server.request_admission_counters().confirmed_admitted_total,
        2
    );
    dispatch(&server, dcc(3), None, None).await; // recovery partition full
    let abort = observed(&mut started).await;
    assert_eq!(
        server
            .request_admission_counters()
            .recovery_overloaded_total,
        2
    );
    assert_eq!(server.dcc_outcome_counters().policy_denied_total, 1);
    assert_eq!(capture.0.lock().unwrap().len(), 1);
    begin_stop(&mut server).await;
    ordinary.await.unwrap();
    recovery.await.unwrap();
    abort.await.unwrap();
    reused_abort.await.unwrap();
    dispatch(&server, dcc(4), None, None).await;
    assert_eq!(
        server
            .request_admission_counters()
            .confirmed_shutdown_rejected_total,
        1
    );
    assert_eq!(server.dcc_outcome_counters().policy_denied_total, 1);
    assert_eq!(capture.0.lock().unwrap().len(), 1);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn disable_initiation_still_admits_requests_and_answers_overload_with_abort() {
    // The server refuses DISABLE, so DISABLE_INITIATION is the only state DCC
    // leaves it in: admission and the overload Abort carry on unchanged.
    let (mut server, _tx, mut started) = small_fixture().await;
    server.comm_state.set_for_test(DccState::DisableInitiation);
    dispatch(&server, request(1), None, None).await;
    let answered = observed(&mut started).await;
    dispatch(&server, request(2), None, None).await; // capacity is one
    let aborted = observed(&mut started).await;
    let counters = server.request_admission_counters();
    assert_eq!(counters.confirmed_admitted_total, 1);
    assert_eq!(counters.confirmed_overloaded_total, 1);
    assert_eq!(counters.abort_admitted_total, 1);
    {
        let frames = held_sends(&server).frames.lock().unwrap();
        assert!(
            matches!(&frames[..], [Apdu::Error(_) | Apdu::Reject(_), Apdu::Abort(abort)]
                if abort.invoke_id == 2 && abort.abort_reason == AbortReason::OUT_OF_RESOURCES),
            "{frames:?}"
        );
    }
    held_sends(&server).release.notify_waiters();
    answered.await.unwrap();
    aborted.await.unwrap();
    wait_reaped(&server).await;
    dispatch(&server, who_is(), None, None).await;
    let i_am = observed(&mut started).await;
    assert_eq!(
        server
            .request_admission_counters()
            .unconfirmed_admitted_total,
        1
    );
    assert!(matches!(
        held_sends(&server).frames.lock().unwrap().last(),
        Some(Apdu::UnconfirmedRequest(req)) if req.service_choice == UnconfirmedServiceChoice::I_AM
    ));
    held_sends(&server).release.notify_waiters();
    i_am.await.unwrap();
    server.stop().await.unwrap();
}

#[tokio::test]
async fn disable_initiation_still_admits_life_safety_operations_and_answers_overload_with_abort() {
    // LifeSafetyOperation has its own admission and replay path in dispatch;
    // DISABLE_INITIATION leaves it, its overload Abort and its replay alone.
    let executions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = Arc::clone(&executions);
    let (mut server, _tx, mut started) = fixture_with_config(
        "admission",
        ServerConfig {
            request_admission_policy: RequestAdmissionPolicy {
                max_confirmed_in_flight: 1,
                confirmed_recovery_reserve: 0,
                ..Default::default()
            },
            policy: Some(Arc::new(crate::FnPolicy::new().life_safety_operation(
                move |_| {
                    counted.fetch_add(1, Ordering::Relaxed);
                    true
                },
            ))),
            ..ServerConfig::default()
        },
    )
    .await;
    server.comm_state.set_for_test(DccState::DisableInitiation);
    dispatch(&server, life_safety(1), None, None).await;
    let answered = observed(&mut started).await;
    dispatch(&server, life_safety(2), None, None).await; // capacity is one
    let aborted = observed(&mut started).await;
    let counters = server.request_admission_counters();
    assert_eq!(counters.confirmed_admitted_total, 1);
    assert_eq!(counters.confirmed_overloaded_total, 1);
    assert_eq!(counters.abort_admitted_total, 1);
    let answer = {
        let frames = held_sends(&server).frames.lock().unwrap();
        assert!(
            matches!(&frames[..], [Apdu::SimpleAck(ack), Apdu::Abort(abort)]
                if ack.invoke_id == 1
                    && ack.service_choice == ConfirmedServiceChoice::LIFE_SAFETY_OPERATION
                    && abort.invoke_id == 2
                    && abort.abort_reason == AbortReason::OUT_OF_RESOURCES),
            "{frames:?}"
        );
        frames[0].clone()
    };
    assert_eq!(executions.load(Ordering::Relaxed), 1);
    held_sends(&server).release.notify_waiters();
    answered.await.unwrap();
    aborted.await.unwrap();
    wait_reaped(&server).await;
    // A retransmission of the executed request is answered from the replay
    // window, not dropped and not run again. Dispatch sends a replay inline,
    // so release the held send beside it.
    let replay = dispatch(&server, life_safety(1), None, None);
    let check = async {
        let replayed = observed(&mut started).await;
        assert_eq!(
            held_sends(&server).frames.lock().unwrap().last(),
            Some(&answer)
        );
        held_sends(&server).release.notify_waiters();
        replayed.await.unwrap();
    };
    tokio::join!(replay, check);
    assert_eq!(
        server.request_admission_counters().confirmed_admitted_total,
        1
    );
    assert_eq!(executions.load(Ordering::Relaxed), 1);
    server.stop().await.unwrap();
}
