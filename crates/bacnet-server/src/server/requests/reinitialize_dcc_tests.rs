//! Restart acceptance ends DISABLE_INITIATION independently of reply delivery.
use super::*;
use bacnet_services::device_mgmt::DeviceCommunicationControlRequest;
use bacnet_types::enums::EnableDisable;

struct Fixture {
    services: RequestServices<BipTransport>,
    tracker: Arc<ConfirmedRequestTracker>,
    tasks: Arc<crate::server::request_tasks::RequestTasks>,
}
impl Fixture {
    fn new(config: ServerConfig) -> Self {
        let network = Arc::new(NetworkLayer::new(BipTransport::new(
            Ipv4Addr::LOCALHOST,
            0,
            Ipv4Addr::BROADCAST,
        )));
        Self {
            services: RequestServices::for_test(network, config),
            tracker: Arc::new(ConfirmedRequestTracker::default()),
            tasks: Arc::default(),
        }
    }
    async fn dispatch(
        &self,
        service: ConfirmedServiceChoice,
        data: Bytes,
        fail_reply: bool,
    ) -> Option<Apdu> {
        let request = ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 480,
            invoke_id: 42,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: service,
            service_request: data,
        };
        let (tx, rx) = oneshot::channel();
        let rx = if fail_reply {
            drop(rx);
            None
        } else {
            Some(rx)
        };
        BACnetServer::<BipTransport>::handle_confirmed_request(
            &self.services,
            &self.tracker,
            &self.tasks.spawner(),
            &SOURCE_MAC,
            None,
            request,
            Some(tx),
        )
        .await;
        match rx {
            Some(rx) => Some(decode_apdu(decode_npdu(rx.await.unwrap()).unwrap().payload).unwrap()),
            None => None,
        }
    }
    async fn restart(&self, state: ReinitializedState, fail_reply: bool) -> Option<Apdu> {
        self.dispatch(
            ConfirmedServiceChoice::REINITIALIZE_DEVICE,
            request_data(state, None),
            fail_reply,
        )
        .await
    }
    async fn disable(&self, minutes: Option<u16>) {
        let mut data = BytesMut::new();
        DeviceCommunicationControlRequest {
            time_duration: minutes,
            enable_disable: EnableDisable::DISABLE_INITIATION,
            password: None,
        }
        .encode(&mut data)
        .unwrap();
        assert!(matches!(
            self.dispatch(
                ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
                data.freeze(),
                false
            )
            .await,
            Some(Apdu::SimpleAck(_))
        ));
    }
}

fn accepting() -> ServerConfig {
    ServerConfig {
        dcc_policy: DccPolicy::LegacyPermissive,
        on_reinitialize: Some(Arc::new(|_, _| Ok(()))),
        ..Default::default()
    }
}

#[tokio::test(start_paused = true)]
async fn reinitialize_accepted_restart_enables_communication() {
    for restart in [ReinitializedState::WARMSTART, ReinitializedState::COLDSTART] {
        let f = Fixture::new(accepting());
        f.disable(None).await;
        assert_eq!(f.services.comm_state.get(), DccState::DisableInitiation);
        assert!(matches!(
            f.restart(restart, false).await,
            Some(Apdu::SimpleAck(_))
        ));
        assert_eq!(
            f.services.comm_state.get(),
            DccState::Enable,
            "accepted {restart:?} must end DISABLE_INITIATION"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn reinitialize_restart_cancels_expiry_without_a_dcc_outcome() {
    for restart in [ReinitializedState::WARMSTART, ReinitializedState::COLDSTART] {
        for fail_reply in [false, true] {
            let f = Fixture::new(accepting());
            f.disable(Some(1)).await;
            let old = f
                .services
                .dcc_timer
                .lock()
                .await
                .as_ref()
                .unwrap()
                .abort_handle();
            let outcomes = f.services.dcc_outcomes.snapshot();
            let reply = f.restart(restart, fail_reply).await;
            assert!(fail_reply || matches!(reply, Some(Apdu::SimpleAck(_))));
            assert_eq!(f.services.comm_state.get(), DccState::Enable);
            assert!(old.is_finished());
            assert!(f.services.dcc_timer.lock().await.is_none());
            assert_eq!(f.services.dcc_outcomes.snapshot(), outcomes);
            // The superseded deadline must not override a later indefinite disable.
            f.disable(None).await;
            tokio::time::advance(Duration::from_secs(61)).await;
            assert_eq!(f.services.comm_state.get(), DccState::DisableInitiation);
        }
    }
}

fn held_timer() -> (tokio::task::JoinHandle<()>, oneshot::Receiver<()>) {
    let (tx, rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        let _resource = tx;
        std::future::pending::<()>().await;
    });
    (task, rx)
}

#[tokio::test(start_paused = true)]
async fn reinitialize_cancelled_after_acceptance_keeps_commit_and_timer_ownership() {
    for restart in [ReinitializedState::WARMSTART, ReinitializedState::COLDSTART] {
        let (mut config, received) = recording_config(None, || Ok(()));
        config.dcc_policy = DccPolicy::LegacyPermissive;
        let f = Fixture::new(config);
        f.services
            .comm_state
            .set_for_test(DccState::DisableInitiation);
        let (old, mut resource) = held_timer();
        let id = old.id();
        **f.services.dcc_timer.lock().await = Some(old);
        {
            let request = f.restart(restart, false);
            tokio::pin!(request);
            std::future::poll_fn(|cx| {
                assert!(std::future::Future::poll(request.as_mut(), cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            // No runtime yield: the request is waiting to join its aborted timer,
            // but acceptance already enabled communication before any reply.
            assert_eq!(received.lock().unwrap().len(), 1);
            assert_eq!(f.services.comm_state.get(), DccState::Enable);
        }
        assert_eq!(f.services.dcc_timer.lock().await.as_ref().unwrap().id(), id);
        assert_eq!(
            resource.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        );
        f.disable(None).await;
        assert_eq!(
            resource.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        );
        assert!(f.services.dcc_timer.lock().await.is_none());
        tokio::time::advance(Duration::from_secs(61)).await;
        assert_eq!(f.services.comm_state.get(), DccState::DisableInitiation);
    }
}

#[tokio::test(start_paused = true)]
async fn reinitialize_cancelled_before_acceptance_preserves_disable_and_timer() {
    let (config, received) = recording_config(None, || Ok(()));
    let f = Fixture::new(config);
    f.services
        .comm_state
        .set_for_test(DccState::DisableInitiation);
    let (old, mut resource) = held_timer();
    let id = old.id();
    **f.services.dcc_timer.lock().await = Some(old);
    let database = f.services.db.write().await;
    {
        let request = f.restart(ReinitializedState::WARMSTART, false);
        tokio::pin!(request);
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(request.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
    }
    assert!(received.lock().unwrap().is_empty());
    assert_eq!(f.services.comm_state.get(), DccState::DisableInitiation);
    // Let an accidentally aborted task run so its resource would expose that.
    tokio::task::yield_now().await;
    assert_eq!(
        resource.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    );
    let mut slot = f.services.dcc_timer.lock().await;
    assert_eq!(slot.as_ref().unwrap().id(), id);
    drop(database);
    crate::server::dcc_timer::cancel(&mut slot).await;
}

#[tokio::test(start_paused = true)]
async fn reinitialize_non_restart_and_failed_requests_preserve_disable_and_timer() {
    let mut cases: Vec<(ServerConfig, Bytes, bool)> = DEFINED_STATES[2..]
        .iter()
        .map(|&state| (accepting(), request_data(state, None), true))
        .collect();
    cases.push((
        ServerConfig::default(),
        request_data(ReinitializedState::WARMSTART, None),
        false,
    ));
    for supplied in [None, Some("wrong")] {
        let mut config = accepting();
        config.reinit_password = Some("required".into());
        cases.push((
            config,
            request_data(ReinitializedState::COLDSTART, supplied),
            false,
        ));
    }
    for state in UNDEFINED_STATES {
        cases.push((accepting(), request_data(state, None), false));
    }
    cases.push((accepting(), Bytes::new(), false));
    for config in [
        recording_config(None, || {
            Err(Error::Protocol {
                class: ErrorClass::DEVICE.to_raw() as u32,
                code: ErrorCode::CONFIGURATION_IN_PROGRESS.to_raw() as u32,
            })
        })
        .0,
        recording_config(None, || panic!("refused restart")).0,
    ] {
        cases.push((
            config,
            request_data(ReinitializedState::WARMSTART, None),
            false,
        ));
    }
    for (config, data, accepted) in cases {
        let f = Fixture::new(config);
        f.services
            .comm_state
            .set_for_test(DccState::DisableInitiation);
        let (old, mut resource) = held_timer();
        let id = old.id();
        **f.services.dcc_timer.lock().await = Some(old);
        let outcomes = f.services.dcc_outcomes.snapshot();
        let reply = f
            .dispatch(ConfirmedServiceChoice::REINITIALIZE_DEVICE, data, false)
            .await;
        assert_eq!(matches!(reply, Some(Apdu::SimpleAck(_))), accepted);
        assert_eq!(f.services.comm_state.get(), DccState::DisableInitiation);
        assert_eq!(f.services.dcc_outcomes.snapshot(), outcomes);
        tokio::task::yield_now().await;
        assert_eq!(
            resource.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        );
        let mut slot = f.services.dcc_timer.lock().await;
        assert_eq!(slot.as_ref().unwrap().id(), id);
        crate::server::dcc_timer::cancel(&mut slot).await;
    }
}
