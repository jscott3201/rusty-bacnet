use super::*;
use crate::server::{
    request_tasks::RequestTasks, DccDisableRateLimit, DccSource, DccSourceRestriction,
};
use bacnet_services::device_mgmt::DeviceCommunicationControlRequest;
use bacnet_types::enums::EnableDisable;
use std::future::Future;
use tokio::time::{advance, Duration};

struct Fixture {
    config: ServerConfig,
    tasks: Arc<RequestTasks>,
    timer: Arc<Mutex<Option<JoinHandle<()>>>>,
    state: Arc<AtomicU8>,
    outcomes: dcc_outcomes::DccOutcomes,
}

impl Fixture {
    fn new(limit: Option<DccDisableRateLimit>) -> Self {
        let config = ServerConfig {
            dcc_policy: DccPolicy::LegacyPermissive,
            dcc_disable_rate_limit: limit,
            ..Default::default()
        };
        Self {
            tasks: RequestTasks::for_server(&config).unwrap(),
            config,
            timer: Arc::new(Mutex::new(None)),
            state: Arc::new(AtomicU8::new(0)),
            outcomes: Default::default(),
        }
    }

    async fn send(
        &self,
        mode: EnableDisable,
        duration: Option<u16>,
        password: Option<&str>,
        peer: u8,
        routed: bool,
    ) -> Apdu {
        let mut data = BytesMut::new();
        DeviceCommunicationControlRequest {
            enable_disable: mode,
            time_duration: duration,
            password: password.map(str::to_owned),
        }
        .encode(&mut data)
        .unwrap();
        self.raw(data.freeze(), peer, routed).await
    }

    async fn raw(&self, data: Bytes, peer: u8, routed: bool) -> Apdu {
        let req = ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 480,
            invoke_id: peer,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
            service_request: data,
        };
        let source = routed.then(|| NpduAddress {
            network: u16::from(peer) + 1,
            mac_address: MacAddr::from_slice(&[peer]),
        });
        response::<BipTransport>(
            &self.timer,
            &self.state,
            &self.outcomes,
            &self.config,
            &req,
            &[peer],
            source.as_ref(),
            &self.tasks.spawner(),
        )
        .await
    }

    async fn disable(&self, peer: u8) -> Apdu {
        self.send(
            EnableDisable::DISABLE_INITIATION,
            None,
            None,
            peer,
            peer.is_multiple_of(2),
        )
        .await
    }

    async fn cleanup(&self) {
        crate::server::dcc_timer::cancel(&mut *self.timer.lock().await).await;
    }
}

fn denied(response: Apdu) {
    assert!(
        matches!(response, Apdu::Error(e) if e.error_class == ErrorClass::SERVICES && e.error_code == ErrorCode::SERVICE_REQUEST_DENIED && e.service_choice == ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL)
    );
}

fn accepted(response: Apdu) {
    assert!(matches!(response, Apdu::SimpleAck(_)));
}

#[tokio::test(start_paused = true)]
async fn dcc_disable_rate_denial_has_one_existing_debug_event_and_counter() {
    use tracing::instrument::WithSubscriber;
    let fixture = Fixture::new(Some(DccDisableRateLimit {
        capacity: 1,
        ..Default::default()
    }));
    accepted(fixture.disable(1).await);
    let capture = dcc_outcomes::trace_tests::Capture::default();
    denied(
        fixture
            .send(EnableDisable::DISABLE_INITIATION, Some(5), None, 42, true)
            .with_subscriber(capture.clone())
            .await,
    );
    assert_eq!(fixture.outcomes.snapshot().policy_denied_total, 1);
    let records = capture.0.lock().unwrap();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record["outcome"], "policy_denied");
    assert_eq!(record["invoke_id"], "42");
    assert_eq!(record["service"], "17");
    assert_eq!(record["decoded_mode"], "2");
    assert_eq!(record["duration_minutes"], "5");
    assert_eq!(record.len(), 11);
}

#[tokio::test(start_paused = true)]
async fn dcc_disable_rate_global_identity_enable_exemption_and_native_reset() {
    let fixture = Fixture::new(Some(Default::default()));
    for peer in 1..=3 {
        accepted(fixture.disable(peer).await);
    }
    denied(fixture.disable(4).await);
    advance(Duration::from_secs(10)).await;
    for peer in 5..=25 {
        accepted(
            fixture
                .send(EnableDisable::ENABLE, None, None, peer, true)
                .await,
        );
        denied(fixture.disable(peer).await);
    }
    advance(Duration::from_secs(10)).await;
    accepted(fixture.disable(26).await);
    denied(fixture.disable(27).await);
    assert_eq!(fixture.outcomes.snapshot().accepted_total, 25);
    assert_eq!(fixture.outcomes.snapshot().policy_denied_total, 23);
    // Closing the same owner never resets the bucket; new native owner does.
    fixture.tasks.close();
    assert!(!fixture.tasks.spawner().admit_dcc_disable());
    let fresh = Fixture::new(Some(Default::default()));
    for peer in 1..=3 {
        accepted(fresh.disable(peer).await);
    }
    denied(fresh.disable(4).await);
    let unlimited = Fixture::new(None);
    for peer in 0..100 {
        accepted(unlimited.disable(peer).await);
    }
}

#[tokio::test(start_paused = true)]
async fn dcc_disable_rate_earlier_failures_do_not_charge() {
    let mut fixture = Fixture::new(Some(Default::default()));
    fixture.config.dcc_password = Some("required".into());
    fixture.config.dcc_policy = DccPolicy::RequirePassword;
    for _ in 0..10 {
        assert!(matches!(
            fixture.raw(Bytes::from_static(&[0x19]), 1, false).await,
            Apdu::Error(_)
        ));
        assert!(
            matches!(fixture.send(EnableDisable::DISABLE_INITIATION, None, Some("wrong"), 1, false).await,
            Apdu::Error(e) if e.error_class == ErrorClass::SECURITY && e.error_code == ErrorCode::PASSWORD_FAILURE)
        );
        denied(
            fixture
                .send(EnableDisable::DISABLE, None, Some("required"), 1, false)
                .await,
        );
        assert!(matches!(
            fixture
                .send(EnableDisable::from_raw(3), None, Some("required"), 1, false)
                .await,
            Apdu::Error(_)
        ));
        fixture.config.dcc_policy = DccPolicy::DenyAll;
        denied(
            fixture
                .send(
                    EnableDisable::DISABLE_INITIATION,
                    None,
                    Some("required"),
                    1,
                    false,
                )
                .await,
        );
        fixture.config.dcc_policy = DccPolicy::RequirePassword;
        fixture.config.dcc_source_restriction =
            Some(DccSourceRestriction::new(vec![DccSource::Direct(vec![2])]).unwrap());
        denied(
            fixture
                .send(
                    EnableDisable::DISABLE_INITIATION,
                    None,
                    Some("required"),
                    1,
                    false,
                )
                .await,
        );
        fixture.config.dcc_source_restriction = None;
    }
    for _ in 0..3 {
        accepted(
            fixture
                .send(
                    EnableDisable::DISABLE_INITIATION,
                    None,
                    Some("required"),
                    1,
                    false,
                )
                .await,
        );
    }
    denied(
        fixture
            .send(
                EnableDisable::DISABLE_INITIATION,
                None,
                Some("required"),
                1,
                false,
            )
            .await,
    );
    let counters = fixture.outcomes.snapshot();
    assert_eq!(
        (
            counters.accepted_total,
            counters.policy_denied_total,
            counters.password_failure_total,
            counters.deprecated_denied_total,
            counters.malformed_total
        ),
        (3, 21, 10, 10, 20)
    );
}

#[tokio::test(start_paused = true)]
async fn dcc_disable_rate_admitted_cancellation_consumes_before_timer_lock() {
    let fixture = Fixture::new(Some(DccDisableRateLimit {
        capacity: 1,
        ..Default::default()
    }));
    let slot = fixture.timer.lock().await;
    let mut request = Box::pin(fixture.disable(1));
    assert!(matches!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(request.as_mut().poll(cx))).await,
        std::task::Poll::Pending
    ));
    drop(request);
    assert_eq!(fixture.state.load(Ordering::Acquire), 0);
    assert_eq!(fixture.outcomes.snapshot().accepted_total, 0);
    // A timeout cannot auto-advance this paused clock: poll once, requiring Ready.
    let mut denied_request = Box::pin(fixture.disable(2));
    let result =
        std::future::poll_fn(|cx| std::task::Poll::Ready(denied_request.as_mut().poll(cx))).await;
    let std::task::Poll::Ready(result) = result else {
        panic!("rate denial waited for timer lock")
    };
    denied(result);
    assert!(slot.is_none());
    assert_eq!(fixture.outcomes.snapshot().policy_denied_total, 1);
    drop(slot);
    advance(Duration::from_secs(20)).await;
    accepted(fixture.disable(3).await);
}

#[tokio::test(start_paused = true)]
async fn dcc_disable_rate_concurrent_handlers_admit_only_burst_before_commit() {
    let fixture = Fixture::new(Some(Default::default()));
    let slot = fixture.timer.lock().await;
    let mut pending = Vec::new();
    for peer in 1..=32 {
        let mut request = Box::pin(fixture.disable(peer));
        match std::future::poll_fn(|cx| std::task::Poll::Ready(request.as_mut().poll(cx))).await {
            std::task::Poll::Pending => pending.push(request),
            std::task::Poll::Ready(result) => denied(result),
        }
    }
    assert_eq!(pending.len(), 3);
    assert_eq!(fixture.outcomes.snapshot().accepted_total, 0);
    assert_eq!(fixture.outcomes.snapshot().policy_denied_total, 29);
    drop(slot);
    for request in pending {
        accepted(request.await);
    }
    assert_eq!(fixture.outcomes.snapshot().accepted_total, 3);
    denied(fixture.disable(33).await);
}

#[tokio::test(start_paused = true)]
async fn dcc_disable_rate_denial_preserves_live_timer_racing_expiry() {
    for pending_expiry in [false, true] {
        let fixture = Fixture::new(Some(DccDisableRateLimit {
            capacity: 1,
            refill_interval_ms: 120_000,
        }));
        accepted(
            fixture
                .send(EnableDisable::DISABLE_INITIATION, Some(1), None, 1, false)
                .await,
        );
        tokio::task::yield_now().await;
        let slot = fixture.timer.lock().await;
        let id = slot.as_ref().unwrap().id();
        advance(Duration::from_secs(if pending_expiry { 60 } else { 30 })).await;
        for duration in [None, Some(0), Some(1), Some(5)] {
            let mut request =
                Box::pin(fixture.send(EnableDisable::DISABLE_INITIATION, duration, None, 2, true));
            let result =
                std::future::poll_fn(|cx| std::task::Poll::Ready(request.as_mut().poll(cx))).await;
            let std::task::Poll::Ready(result) = result else {
                panic!("rate denial waited for timer lock")
            };
            denied(result);
            assert_eq!(slot.as_ref().unwrap().id(), id);
            assert_eq!(fixture.state.load(Ordering::Acquire), 2);
        }
        drop(slot);
        if !pending_expiry {
            advance(Duration::from_secs(30)).await;
        }
        let task = fixture.timer.lock().await.take().unwrap();
        task.await.unwrap();
        assert_eq!(fixture.state.load(Ordering::Acquire), 0);
    }
}

#[tokio::test(start_paused = true)]
async fn dcc_disable_rate_authorized_timer_parity_and_repeated_charge() {
    for duration in [None, Some(0), Some(1)] {
        let fixture = Fixture::new(Some(Default::default()));
        for _ in 0..3 {
            accepted(
                fixture
                    .send(EnableDisable::DISABLE_INITIATION, duration, None, 1, false)
                    .await,
            );
            assert_eq!(fixture.state.load(Ordering::Acquire), 2);
            assert_eq!(fixture.timer.lock().await.is_some(), duration.is_some());
        }
        denied(
            fixture
                .send(EnableDisable::DISABLE_INITIATION, duration, None, 1, false)
                .await,
        );
        accepted(
            fixture
                .send(EnableDisable::ENABLE, duration, None, 1, false)
                .await,
        );
        assert_eq!(fixture.state.load(Ordering::Acquire), 0);
        // Preserve the existing ENABLE-duration behavior, not a protocol correction.
        assert_eq!(fixture.timer.lock().await.is_some(), duration.is_some());
        denied(fixture.disable(2).await);
        fixture.cleanup().await;
    }
}
