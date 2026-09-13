use super::*;
use crate::server::dcc_outcomes::trace_tests::Capture;
use bacnet_services::device_mgmt::DeviceCommunicationControlRequest;
use bacnet_types::enums::EnableDisable;
use std::future::{poll_fn, Future};
use std::task::Poll;
use tracing::instrument::WithSubscriber;

fn request(mode: u32, password: Option<&str>, duration: Option<u16>) -> ConfirmedRequestPdu {
    let mut data = BytesMut::new();
    DeviceCommunicationControlRequest {
        time_duration: duration,
        enable_disable: EnableDisable::from_raw(mode),
        password: password.map(str::to_owned),
    }
    .encode(&mut data)
    .unwrap();
    ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 480,
        invoke_id: 42,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
        service_request: data.freeze(),
    }
}

async fn handle(server: &BACnetServer<HeldTransport>, req: ConfirmedRequestPdu) {
    handle_source(server, req, &[1], None).await;
}

async fn handle_source(
    server: &BACnetServer<HeldTransport>,
    req: ConfirmedRequestPdu,
    mac: &[u8],
    source: Option<NpduAddress>,
) {
    BACnetServer::handle_admitted_confirmed_request(
        &server.db,
        &server.network,
        &server.cov_table,
        &server.seg_ack_senders,
        &server.seg_send_permits,
        &server.cov_in_flight,
        &server.server_tsm,
        &server.notification_transactions,
        &server.device_bindings,
        &server.comm_state,
        &server.dcc_timer,
        &server.dcc_outcomes,
        &server.mutation_decisions,
        &server.config,
        &server.request_tasks.spawner(),
        mac,
        source,
        req,
        None,
    )
    .await;
}

#[tokio::test(start_paused = true)]
async fn dcc_source_outcomes_exactly_once_precedence_and_malformed_routing() {
    for (mode, password, expected) in [
        (0, Some("required"), "policy_denied"),
        (2, Some("required"), "policy_denied"),
        (1, Some("required"), "deprecated_denied"),
        (1, None, "password_failure"),
        (99, Some("required"), "malformed"),
    ] {
        for (network, address) in [
            (7, vec![2]),
            (0, vec![1]),
            (65535, vec![1]),
            (7, vec![]),
            (7, vec![1; 256]),
        ] {
            let (mut server, _, mut started) = fixture_with_config(
                "source outcomes",
                ServerConfig {
                    dcc_policy: DccPolicy::RequirePassword,
                    dcc_password: Some("required".into()),
                    dcc_source_restriction: Some(
                        DccSourceRestriction::new(vec![DccSource::Direct(vec![1])]).unwrap(),
                    ),
                    ..Default::default()
                },
            )
            .await;
            let capture = Capture::default();
            {
                let future = handle_source(
                    &server,
                    request(mode, password, None),
                    &[1],
                    Some(NpduAddress {
                        network,
                        mac_address: MacAddr::from_slice(&address),
                    }),
                )
                .with_subscriber(capture.clone());
                tokio::pin!(future);
                // Valid response routing blocks on transport; malformed routing can
                // fail response encoding immediately. Both follow completed denial.
                poll_fn(|cx| {
                    let _ = future.as_mut().poll(cx);
                    Poll::Ready(())
                })
                .await;
                let counts = server.dcc_outcome_counters();
                assert_eq!(counts.accepted_total, 0);
                assert_eq!(
                    counts.policy_denied_total,
                    u64::from(expected == "policy_denied")
                );
                assert_eq!(
                    counts.password_failure_total,
                    u64::from(expected == "password_failure")
                );
                assert_eq!(
                    counts.deprecated_denied_total,
                    u64::from(expected == "deprecated_denied")
                );
                assert_eq!(counts.malformed_total, u64::from(expected == "malformed"));
                assert_eq!(server.comm_state(), 0);
                assert!(server.dcc_timer.lock().await.is_none());
                let events = capture.0.lock().unwrap();
                assert_eq!(events.len(), 1);
                assert_eq!(events[0]["outcome"], expected);
                assert_eq!(events[0]["source_kind"], "claimed_routed");
            }
            let _ = started.try_recv();
            let counts = server.dcc_outcome_counters();
            server.stop().await.unwrap();
            assert_eq!(server.dcc_outcome_counters(), counts);
            assert_eq!(capture.0.lock().unwrap().len(), 1);
        }
    }
}

async fn poll_pending(future: std::pin::Pin<&mut impl Future<Output = ()>>) {
    let mut future = future;
    poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn dcc_outcomes_exact_precedence_before_response_and_secret_redaction() {
    // Acceptance tests for the new API, not a claimed baseline behavioral RED.
    for (policy, mode, password, expected) in [
        (
            DccPolicy::LegacyPermissive,
            2,
            "sentinel-correct",
            "accepted",
        ),
        (
            DccPolicy::RequirePassword,
            0,
            "sentinel-correct",
            "accepted",
        ),
        (DccPolicy::DenyAll, 0, "sentinel-correct", "policy_denied"),
        (DccPolicy::DenyAll, 2, "sentinel-correct", "policy_denied"),
        (
            DccPolicy::DenyAll,
            1,
            "sentinel-correct",
            "deprecated_denied",
        ),
        (DccPolicy::DenyAll, 1, "sentinel-wrong", "password_failure"),
        (DccPolicy::DenyAll, 99, "sentinel-correct", "malformed"),
        (DccPolicy::DenyAll, 99, "sentinel-wrong", "password_failure"),
    ] {
        let (mut server, _, mut started) = fixture_with_config(
            "outcomes",
            ServerConfig {
                dcc_policy: policy,
                dcc_password: Some("sentinel-correct".into()),
                ..Default::default()
            },
        )
        .await;
        let capture = Capture::default();
        assert_eq!(server.dcc_outcome_counters(), DccOutcomeCounters::default());
        {
            let future = handle(&server, request(mode, Some(password), Some(10)))
                .with_subscriber(capture.clone());
            tokio::pin!(future);
            poll_pending(future.as_mut()).await;
            let completion = started.try_recv().unwrap();
            let counts = server.dcc_outcome_counters();
            assert_eq!(counts.accepted_total, u64::from(expected == "accepted"));
            assert_eq!(
                counts.policy_denied_total,
                u64::from(expected == "policy_denied")
            );
            assert_eq!(
                counts.password_failure_total,
                u64::from(expected == "password_failure")
            );
            assert_eq!(
                counts.deprecated_denied_total,
                u64::from(expected == "deprecated_denied")
            );
            assert_eq!(counts.malformed_total, u64::from(expected == "malformed"));
            assert_eq!(
                server.comm_state(),
                if expected == "accepted" {
                    mode as u8
                } else {
                    0
                }
            );
            {
                let events = capture.0.lock().unwrap();
                assert_eq!(events.len(), 1);
                assert_eq!(events[0]["outcome"], expected);
                assert_eq!(events[0]["decoded_mode"], mode.to_string());
                assert_eq!(events[0]["duration_minutes"], "10");
                assert_eq!(events[0]["invoke_id"], "42");
                assert_eq!(events[0]["service"], "17");
                assert!(!format!("{events:?}").contains("sentinel"));
                assert!(!events[0].keys().any(|key| key.contains("password")));
            }
            // Fail transport after the commit; never revise or double count it.
            server
                .network
                .transport()
                .fail_next
                .store(true, Ordering::Release);
            server.network.transport().release.notify_one();
            future.await;
            completion.await.unwrap();
            assert_eq!(server.dcc_outcome_counters(), counts);
        }
        let counts = server.dcc_outcome_counters();
        server.stop().await.unwrap();
        assert_eq!(server.dcc_outcome_counters(), counts);
        assert_eq!(capture.0.lock().unwrap().len(), 1);
    }
}

#[tokio::test(start_paused = true)]
async fn dcc_outcomes_cancellation_denial_timer_identity_and_decode_failure() {
    let (mut server, _, mut started) = fixture_with_config(
        "cancel",
        ServerConfig {
            dcc_policy: DccPolicy::LegacyPermissive,
            ..Default::default()
        },
    )
    .await;
    let capture = Capture::default();
    let _subscriber = tracing::subscriber::set_default(capture.clone());
    let lock = server.dcc_timer.lock().await;
    {
        let future = handle(&server, request(2, None, Some(10))).with_subscriber(capture.clone());
        tokio::pin!(future);
        poll_pending(future.as_mut()).await;
    }
    assert_eq!(server.dcc_outcome_counters(), DccOutcomeCounters::default());
    assert_eq!(server.comm_state(), 0);
    assert!(capture.0.lock().unwrap().is_empty());
    drop(lock);
    {
        let future = handle(&server, request(2, None, Some(10))).with_subscriber(capture.clone());
        tokio::pin!(future);
        poll_pending(future.as_mut()).await;
        started.try_recv().unwrap();
        // Drop while response is blocked, after the live commit.
    }
    assert_eq!(server.dcc_outcome_counters().accepted_total, 1);
    tokio::task::yield_now().await; // Arm the accepted timer before advancing time.
    server.config.dcc_policy = DccPolicy::DenyAll;
    let lock = server.dcc_timer.lock().await;
    let identity = lock.as_ref().unwrap().id();
    for _ in 0..2 {
        let future = handle(&server, request(1, None, None)).with_subscriber(capture.clone());
        tokio::pin!(future);
        poll_pending(future.as_mut()).await;
        started.try_recv().unwrap();
        assert_eq!(lock.as_ref().unwrap().id(), identity);
    }
    for mode in [0, 2] {
        let future = handle(&server, request(mode, None, None)).with_subscriber(capture.clone());
        tokio::pin!(future);
        poll_pending(future.as_mut()).await;
        started.try_recv().unwrap();
        assert_eq!(lock.as_ref().unwrap().id(), identity);
        assert_eq!(server.comm_state(), 2);
    }
    // Invalid UTF-8 password: no partially decoded metadata or error text logged.
    let mut malformed = request(0, None, None);
    malformed.service_request = Bytes::from_static(b"\x19\x00\x2d\x0b\x00\xffsentinel!");
    {
        let future = handle(&server, malformed).with_subscriber(capture.clone());
        tokio::pin!(future);
        poll_pending(future.as_mut()).await;
        started.try_recv().unwrap();
    }
    {
        let records = capture.0.lock().unwrap();
        assert_eq!(records.len(), 6);
        assert_eq!(records[5]["outcome"], "malformed");
        assert!(!records[5].contains_key("decoded_mode"));
        assert!(!records[5].contains_key("duration_minutes"));
        assert!(!format!("{records:?}").contains("sentinel"));
    }
    drop(lock);
    let counts = server.dcc_outcome_counters();
    assert_eq!(counts.deprecated_denied_total, 2);
    assert_eq!(counts.malformed_total, 1);
    assert_eq!(counts.policy_denied_total, 2);
    tokio::time::advance(Duration::from_secs(601)).await;
    tokio::task::yield_now().await;
    assert_eq!(server.dcc_outcome_counters(), counts);
    assert_eq!(server.comm_state(), 0);
    assert_eq!(capture.0.lock().unwrap().len(), 6);
    let (mut other, _, _) = fixture().await;
    assert_eq!(other.dcc_outcome_counters(), DccOutcomeCounters::default());
    other.stop().await.unwrap();
    server.stop().await.unwrap();
}
