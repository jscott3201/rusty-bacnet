use super::*;

use bacnet_encoding::{apdu::decode_apdu, npdu::decode_npdu};
use bacnet_services::device_mgmt::DeviceCommunicationControlRequest;
use bacnet_types::enums::EnableDisable;
use tokio::time::{Duration, advance};

#[path = "dcc_local_network_tests.rs"]
mod local_network;
#[path = "dcc_server_policy_tests.rs"]
mod server_policy;

#[tokio::test(start_paused = true)]
async fn dcc_source_exact_full_bytes_and_fail_closed_wire() {
    use crate::server::{DccSource, DccSourceRestriction};
    let direct = vec![127, 0, 0, 1, 0xba, 0xc0];
    // The longest routed source the NPDU codec carries (#1141); the reply
    // goes back to it as DNET/DADR.
    let long = vec![42; bacnet_npdu::MAX_MAC_LEN];
    let restrictions = [
        None,
        Some(DccSourceRestriction::new(vec![]).unwrap()),
        Some(DccSourceRestriction::new(vec![DccSource::Direct(direct.clone())]).unwrap()),
        Some(
            DccSourceRestriction::new(vec![DccSource::Routed {
                network: 7,
                address: long.clone(),
            }])
            .unwrap(),
        ),
    ];
    for (kind, restriction) in restrictions.into_iter().enumerate() {
        let config = ServerConfig {
            dcc_policy: DccPolicy::RequirePassword,
            dcc_password: Some("required".into()),
            dcc_source_restriction: restriction,
            ..Default::default()
        };
        let mut different_tail = long.clone();
        *different_tail.last_mut().unwrap() = 43;
        for (network, address, exact) in [
            (7, long.clone(), true),
            (8, long.clone(), false),
            (7, different_tail, false),
            (7, long[..long.len() - 1].to_vec(), false),
            (7, direct.clone(), false),
        ] {
            for routed in [false, true] {
                let source = routed.then(|| NpduAddress {
                    network,
                    mac_address: MacAddr::from_slice(&address),
                });
                let state = comm_state_in(DccState::DisableInitiation);
                let timer = Arc::new(Mutex::new(crate::server::dcc_timer::TimerSlot::default()));
                let response = dispatch_wire(
                    &state,
                    &timer,
                    EnableDisable::ENABLE,
                    None,
                    &config,
                    Some("required"),
                    source,
                )
                .await;
                let allowed = kind == 0 || (kind == 2 && !routed) || (kind == 3 && routed && exact);
                if allowed {
                    assert!(matches!(response, Apdu::SimpleAck(_)));
                } else {
                    assert_denied(response);
                }
                assert_eq!(
                    state.get(),
                    if allowed {
                        DccState::Enable
                    } else {
                        DccState::DisableInitiation
                    }
                );
                assert!(timer.lock().await.is_none());
            }
        }
    }
    // Direct matching also uses full bytes, independently of the wire fixture's
    // MAC, up to the longest entry a restriction holds (#1157).
    for length in [1, 6, bacnet_npdu::MAX_MAC_LEN - 1, bacnet_npdu::MAX_MAC_LEN] {
        let address = vec![1; length];
        let restriction =
            DccSourceRestriction::new(vec![DccSource::Direct(address.clone())]).unwrap();
        assert!(restriction.allows(&address, None, None));
        let mut other = address.clone();
        other[length - 1] = 2;
        assert!(!restriction.allows(&other, None, None));
        assert!(!restriction.allows(&address[..length - 1], None, None));
        let mut longer = address.clone();
        longer.push(1);
        assert!(!restriction.allows(&longer, None, None));
        for (network, mac) in [
            (0, address.clone()),
            (65535, address.clone()),
            (7, vec![]),
            (7, vec![1; 256]),
        ] {
            assert!(!restriction.allows(
                &address,
                Some(&NpduAddress {
                    network,
                    mac_address: MacAddr::from_slice(&mac)
                }),
                None,
            ));
        }
    }
}

#[tokio::test(start_paused = true)]
async fn dcc_source_denial_preserves_timer_and_error_precedence() {
    use crate::server::DccSourceRestriction;
    for pending_expiry in [false, true] {
        let state = comm_state_in(DccState::Enable);
        let timer = Arc::new(Mutex::new(crate::server::dcc_timer::TimerSlot::default()));
        assert!(matches!(
            dispatch(&state, &timer, EnableDisable::DISABLE_INITIATION, Some(1)).await,
            Apdu::SimpleAck(_)
        ));
        tokio::task::yield_now().await;
        let slot = timer.lock().await;
        let id = slot.as_ref().unwrap().id();
        advance(Duration::from_secs(if pending_expiry { 60 } else { 30 })).await;
        let config = ServerConfig {
            dcc_policy: DccPolicy::RequirePassword,
            dcc_password: Some("required".into()),
            dcc_source_restriction: Some(DccSourceRestriction::new(vec![]).unwrap()),
            ..Default::default()
        };
        for mode in [
            EnableDisable::ENABLE,
            EnableDisable::DISABLE_INITIATION,
            EnableDisable::DISABLE,
        ] {
            for duration in [None, Some(0), Some(5)] {
                for password in [None, Some("wrong"), Some("required")] {
                    let response =
                        dispatch_wire(&state, &timer, mode, duration, &config, password, None)
                            .await;
                    if password == Some("required") {
                        assert_denied(response);
                    } else {
                        assert!(
                            matches!(response, Apdu::Error(e) if e.error_class == ErrorClass::SECURITY && e.error_code == ErrorCode::PASSWORD_FAILURE)
                        );
                    }
                    assert_eq!(slot.as_ref().unwrap().id(), id);
                    assert_eq!(state.get(), DccState::DisableInitiation);
                }
            }
        }
        drop(slot);
        if !pending_expiry {
            advance(Duration::from_secs(29)).await;
            tokio::task::yield_now().await;
            assert_eq!(state.get(), DccState::DisableInitiation);
            advance(Duration::from_secs(1)).await;
        }
        let task = timer.lock().await.take().unwrap();
        task.await.unwrap();
        assert_eq!(state.get(), DccState::Enable);
    }
}

/// A communication state already in `initial`.
fn comm_state_in(initial: DccState) -> Arc<CommState> {
    let state = Arc::new(CommState::default());
    state.set_for_test(initial);
    state
}

async fn dispatch(
    comm_state: &Arc<CommState>,
    dcc_timer: &Arc<Mutex<crate::server::dcc_timer::TimerSlot>>,
    mode: EnableDisable,
    duration: Option<u16>,
) -> Apdu {
    dispatch_with_config(
        comm_state,
        dcc_timer,
        mode,
        duration,
        &ServerConfig {
            dcc_policy: DccPolicy::LegacyPermissive,
            ..Default::default()
        },
    )
    .await
}

async fn dispatch_with_config(
    comm_state: &Arc<CommState>,
    dcc_timer: &Arc<Mutex<crate::server::dcc_timer::TimerSlot>>,
    mode: EnableDisable,
    duration: Option<u16>,
    config: &ServerConfig,
) -> Apdu {
    dispatch_wire(comm_state, dcc_timer, mode, duration, config, None, None).await
}

async fn dispatch_wire(
    comm_state: &Arc<CommState>,
    dcc_timer: &Arc<Mutex<crate::server::dcc_timer::TimerSlot>>,
    mode: EnableDisable,
    duration: Option<u16>,
    config: &ServerConfig,
    password: Option<&str>,
    source: Option<NpduAddress>,
) -> Apdu {
    let network = Arc::new(NetworkLayer::new(BipTransport::new(
        Ipv4Addr::LOCALHOST,
        0,
        Ipv4Addr::BROADCAST,
    )));
    let mut data = BytesMut::new();
    DeviceCommunicationControlRequest {
        time_duration: duration,
        enable_disable: mode,
        password: password.map(str::to_owned),
    }
    .encode(&mut data)
    .unwrap();
    let request = ConfirmedRequestPdu {
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
    };
    let (tx, rx) = oneshot::channel();
    BACnetServer::<BipTransport>::handle_confirmed_request(
        &RequestServices {
            comm_state: Arc::clone(comm_state),
            dcc_timer: Arc::clone(dcc_timer),
            ..RequestServices::for_test(Arc::clone(&network), config.clone())
        },
        &Arc::new(ConfirmedRequestTracker::default()),
        &Arc::new(crate::server::request_tasks::RequestTasks::default()).spawner(),
        &[127, 0, 0, 1, 0xba, 0xc0],
        source,
        request,
        Some(tx),
    )
    .await;
    let npdu = decode_npdu(rx.await.unwrap()).unwrap();
    decode_apdu(npdu.payload).unwrap()
}

fn assert_denied(apdu: Apdu) {
    let Apdu::Error(error) = apdu else {
        panic!("expected Error, got {apdu:?}")
    };
    assert_eq!(error.invoke_id, 42);
    assert_eq!(
        error.service_choice,
        ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL
    );
    assert_eq!(error.error_class, ErrorClass::SERVICES);
    assert_eq!(error.error_code, ErrorCode::SERVICE_REQUEST_DENIED);
}

#[tokio::test(start_paused = true)]
async fn dcc_default_denies_valid_modes_without_live_mutation() {
    for mode in [EnableDisable::ENABLE, EnableDisable::DISABLE_INITIATION] {
        for initial in [DccState::Enable, DccState::DisableInitiation] {
            for duration in [None, Some(0), Some(1)] {
                let state = comm_state_in(initial);
                let timer = Arc::new(Mutex::new(crate::server::dcc_timer::TimerSlot::default()));
                let response =
                    dispatch_with_config(&state, &timer, mode, duration, &ServerConfig::default())
                        .await;
                assert_eq!(state.get(), initial);
                assert!(timer.lock().await.is_none());
                assert_denied(response);
            }
        }
    }
}

#[tokio::test(start_paused = true)]
async fn dcc_policy_wire_password_precedence_direct_and_routed() {
    for policy in [
        DccPolicy::DenyAll,
        DccPolicy::RequirePassword,
        DccPolicy::LegacyPermissive,
    ] {
        for configured in [None, Some("required")] {
            if policy == DccPolicy::RequirePassword && configured.is_none() {
                continue;
            }
            let config = ServerConfig {
                dcc_policy: policy,
                dcc_password: configured.map(str::to_owned),
                ..Default::default()
            };
            for password in [None, Some("wrong"), Some("required")] {
                for mode in [
                    EnableDisable::ENABLE,
                    EnableDisable::DISABLE_INITIATION,
                    EnableDisable::DISABLE,
                ] {
                    for routed in [false, true] {
                        for duration in [None, Some(0), Some(2)] {
                            let initial = if mode == EnableDisable::ENABLE {
                                DccState::DisableInitiation
                            } else {
                                DccState::Enable
                            };
                            let state = comm_state_in(initial);
                            let timer = Arc::new(Mutex::new(
                                crate::server::dcc_timer::TimerSlot::default(),
                            ));
                            let source = routed.then(|| NpduAddress {
                                network: 7,
                                mac_address: MacAddr::from_slice(&[42]),
                            });
                            let response = dispatch_wire(
                                &state, &timer, mode, duration, &config, password, source,
                            )
                            .await;
                            let bad_password = configured.is_some() && password != configured;
                            if bad_password {
                                assert!(
                                    matches!(response, Apdu::Error(e) if e.error_class == ErrorClass::SECURITY && e.error_code == ErrorCode::PASSWORD_FAILURE)
                                );
                            } else if policy == DccPolicy::DenyAll || mode == EnableDisable::DISABLE
                            {
                                assert_denied(response);
                            } else {
                                assert!(matches!(response, Apdu::SimpleAck(_)));
                                assert_eq!(
                                    state.get(),
                                    if mode == EnableDisable::ENABLE {
                                        DccState::Enable
                                    } else {
                                        DccState::DisableInitiation
                                    }
                                );
                                assert_eq!(timer.lock().await.is_some(), duration.is_some());
                                super::super::super::dcc_timer::cancel(&mut *timer.lock().await)
                                    .await;
                                continue;
                            }
                            assert_eq!(state.get(), initial);
                            assert!(timer.lock().await.is_none());
                        }
                    }
                }
            }
        }
    }
}

#[tokio::test(start_paused = true)]
async fn dcc_default_denials_preserve_timer_even_with_expiry_waiting_for_lock() {
    for pending_expiry in [false, true] {
        let state = comm_state_in(DccState::Enable);
        let timer = Arc::new(Mutex::new(crate::server::dcc_timer::TimerSlot::default()));
        assert!(matches!(
            dispatch(&state, &timer, EnableDisable::DISABLE_INITIATION, Some(1)).await,
            Apdu::SimpleAck(_)
        ));
        tokio::task::yield_now().await;
        let slot = timer.lock().await;
        let id = slot.as_ref().unwrap().id();
        advance(Duration::from_secs(if pending_expiry { 60 } else { 30 })).await;
        tokio::task::yield_now().await;
        for mode in [
            EnableDisable::ENABLE,
            EnableDisable::DISABLE_INITIATION,
            EnableDisable::DISABLE,
        ] {
            for duration in [None, Some(0), Some(5)] {
                for password in [None, Some("required")] {
                    let config = ServerConfig {
                        dcc_password: password.map(str::to_owned),
                        ..Default::default()
                    };
                    // Holding the live lock proves rejection cannot wait for or mutate it.
                    assert_denied(
                        dispatch_wire(&state, &timer, mode, duration, &config, password, None)
                            .await,
                    );
                    assert_eq!(state.get(), DccState::DisableInitiation);
                    assert_eq!(slot.as_ref().unwrap().id(), id);
                }
            }
        }
        drop(slot);
        if !pending_expiry {
            advance(Duration::from_secs(29)).await;
            tokio::task::yield_now().await;
            assert_eq!(state.get(), DccState::DisableInitiation);
            advance(Duration::from_secs(1)).await;
        }
        let task = timer.lock().await.take().unwrap();
        task.await.unwrap();
        assert_eq!(state.get(), DccState::Enable);
    }
}

#[tokio::test(start_paused = true)]
async fn dcc_disable_does_not_replace_cancel_or_extend_active_timer() {
    for rejected_duration in [None, Some(0), Some(1), Some(5)] {
        let state = comm_state_in(DccState::Enable);
        let timer = Arc::new(Mutex::new(crate::server::dcc_timer::TimerSlot::default()));
        assert!(matches!(
            dispatch(&state, &timer, EnableDisable::DISABLE_INITIATION, Some(1)).await,
            Apdu::SimpleAck(_)
        ));
        tokio::task::yield_now().await;
        let timer_id = timer.lock().await.as_ref().unwrap().id();
        advance(Duration::from_secs(30)).await;
        assert_denied(dispatch(&state, &timer, EnableDisable::DISABLE, rejected_duration).await);
        assert_eq!(state.get(), DccState::DisableInitiation);
        assert_eq!(timer.lock().await.as_ref().unwrap().id(), timer_id);
        advance(Duration::from_secs(29)).await;
        tokio::task::yield_now().await;
        assert_eq!(state.get(), DccState::DisableInitiation);
        advance(Duration::from_secs(1)).await;
        let handle = timer.lock().await.take().unwrap();
        handle.await.unwrap();
        assert_eq!(state.get(), DccState::Enable);
    }
}

#[tokio::test(start_paused = true)]
async fn dcc_disable_does_not_create_timer_in_any_state() {
    for initial in [DccState::Enable, DccState::DisableInitiation] {
        let state = comm_state_in(initial);
        let timer = Arc::new(Mutex::new(crate::server::dcc_timer::TimerSlot::default()));
        assert_denied(dispatch(&state, &timer, EnableDisable::DISABLE, Some(1)).await);
        assert!(timer.lock().await.is_none());
        advance(Duration::from_secs(61)).await;
        assert_eq!(state.get(), initial);
    }
}

fn held_timer() -> (JoinHandle<()>, oneshot::Receiver<()>) {
    let (tx, rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        let _resource = tx;
        std::future::pending::<()>().await;
    });
    (handle, rx)
}

#[tokio::test(start_paused = true)]
async fn dcc_require_password_preserves_replacement_expiry_and_enable_timer_semantics() {
    let config = ServerConfig {
        dcc_policy: DccPolicy::RequirePassword,
        dcc_password: Some("required".into()),
        dcc_source_restriction: Some(
            crate::server::DccSourceRestriction::new(vec![crate::server::DccSource::Direct(vec![
                127, 0, 0, 1, 0xba, 0xc0,
            ])])
            .unwrap(),
        ),
        ..Default::default()
    };
    let state = comm_state_in(DccState::Enable);
    let timer = Arc::new(Mutex::new(crate::server::dcc_timer::TimerSlot::default()));
    for (mode, duration) in [
        (EnableDisable::DISABLE_INITIATION, Some(1)),
        (EnableDisable::DISABLE_INITIATION, Some(2)),
        (EnableDisable::ENABLE, Some(2)),
        (EnableDisable::DISABLE_INITIATION, None),
        (EnableDisable::ENABLE, None),
        (EnableDisable::DISABLE_INITIATION, Some(0)),
    ] {
        let previous = timer.lock().await.as_ref().map(JoinHandle::abort_handle);
        assert!(matches!(
            dispatch_wire(
                &state,
                &timer,
                mode,
                duration,
                &config,
                Some("required"),
                None
            )
            .await,
            Apdu::SimpleAck(_)
        ));
        if let Some(previous) = previous {
            assert!(previous.is_finished());
        }
        assert_eq!(
            state.get(),
            if mode == EnableDisable::ENABLE {
                DccState::Enable
            } else {
                DccState::DisableInitiation
            }
        );
        assert_eq!(timer.lock().await.is_some(), duration.is_some());
        tokio::task::yield_now().await;
        if duration == Some(0) {
            let task = timer.lock().await.take().unwrap();
            task.await.unwrap();
            assert_eq!(state.get(), DccState::Enable);
        } else {
            advance(Duration::from_secs(30)).await;
            tokio::task::yield_now().await;
            assert_eq!(
                state.get(),
                if mode == EnableDisable::ENABLE {
                    DccState::Enable
                } else {
                    DccState::DisableInitiation
                }
            );
        }
    }
}

#[tokio::test(start_paused = true)]
async fn dcc_rejection_preserves_pending_expiry_and_password_precedence() {
    let state = comm_state_in(DccState::Enable);
    let timer = Arc::new(Mutex::new(crate::server::dcc_timer::TimerSlot::default()));
    dispatch(&state, &timer, EnableDisable::DISABLE_INITIATION, Some(1)).await;
    tokio::task::yield_now().await;
    let slot = timer.lock().await;
    let id = slot.as_ref().unwrap().id();
    advance(Duration::from_secs(60)).await;
    tokio::task::yield_now().await;
    assert_denied(dispatch(&state, &timer, EnableDisable::DISABLE, Some(5)).await);
    let config = ServerConfig {
        dcc_password: Some("required".into()),
        ..Default::default()
    };
    let response =
        dispatch_with_config(&state, &timer, EnableDisable::DISABLE, Some(5), &config).await;
    let Apdu::Error(error) = response else {
        panic!("expected password failure")
    };
    assert_eq!(error.error_class, ErrorClass::SECURITY);
    assert_eq!(error.error_code, ErrorCode::PASSWORD_FAILURE);
    assert_eq!(slot.as_ref().unwrap().id(), id);
    assert_eq!(state.get(), DccState::DisableInitiation);
    drop(slot);
    let handle = timer.lock().await.take().unwrap();
    handle.await.unwrap();
    assert_eq!(state.get(), DccState::Enable);
    // An expiry that wins first cannot undo a later accepted indefinite state.
    dispatch(&state, &timer, EnableDisable::DISABLE_INITIATION, None).await;
    advance(Duration::from_secs(301)).await;
    assert_eq!(state.get(), DccState::DisableInitiation);
}

#[tokio::test(start_paused = true)]
async fn dcc_replacement_joins_resource_before_ack() {
    let state = comm_state_in(DccState::DisableInitiation);
    let (old, mut resource) = held_timer();
    let finished = old.abort_handle();
    let timer = Arc::new(Mutex::new(crate::server::dcc_timer::TimerSlot(Some(old))));
    assert!(matches!(
        dispatch(&state, &timer, EnableDisable::ENABLE, None).await,
        Apdu::SimpleAck(_)
    ));
    assert!(finished.is_finished());
    assert_eq!(
        resource.try_recv(),
        Err(oneshot::error::TryRecvError::Closed)
    );
    assert!(timer.lock().await.is_none());
    assert_eq!(state.get(), DccState::Enable);
}

#[tokio::test(start_paused = true)]
async fn dcc_cancelled_replacement_retains_join_and_defers_state_commit() {
    let state = comm_state_in(DccState::DisableInitiation);
    let (old, mut resource) = held_timer();
    let id = old.id();
    let timer = Arc::new(Mutex::new(crate::server::dcc_timer::TimerSlot(Some(old))));
    {
        let replacement = dispatch(&state, &timer, EnableDisable::ENABLE, Some(2));
        tokio::pin!(replacement);
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(replacement.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
    }
    assert_eq!(state.get(), DccState::DisableInitiation);
    assert_eq!(timer.lock().await.as_ref().unwrap().id(), id);
    assert_eq!(
        resource.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    );
    assert!(matches!(
        dispatch(&state, &timer, EnableDisable::DISABLE_INITIATION, None).await,
        Apdu::SimpleAck(_)
    ));
    assert_eq!(
        resource.try_recv(),
        Err(oneshot::error::TryRecvError::Closed)
    );
    assert!(timer.lock().await.is_none());
    advance(Duration::from_secs(121)).await;
    assert_eq!(state.get(), DccState::DisableInitiation);
}

#[tokio::test(start_paused = true)]
async fn dcc_concurrent_replacements_serialize_with_pending_expiry() {
    let state = comm_state_in(DccState::Enable);
    let timer = Arc::new(Mutex::new(crate::server::dcc_timer::TimerSlot::default()));
    dispatch(&state, &timer, EnableDisable::DISABLE_INITIATION, Some(1)).await;
    tokio::task::yield_now().await;
    let slot = timer.lock().await;
    let old = slot.as_ref().unwrap().abort_handle();
    let first = dispatch(&state, &timer, EnableDisable::ENABLE, Some(1));
    let second = dispatch(&state, &timer, EnableDisable::DISABLE_INITIATION, Some(2));
    tokio::pin!(first, second);
    // Queue both replacements before expiry, with deterministic FIFO ordering.
    std::future::poll_fn(|cx| {
        assert!(std::future::Future::poll(first.as_mut(), cx).is_pending());
        assert!(std::future::Future::poll(second.as_mut(), cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    advance(Duration::from_secs(60)).await;
    tokio::task::yield_now().await;
    assert_eq!(
        state.get(),
        DccState::DisableInitiation,
        "expiry must share the slot lock"
    );
    drop(slot);
    assert!(matches!(first.await, Apdu::SimpleAck(_)));
    assert!(matches!(second.await, Apdu::SimpleAck(_)));
    assert!(old.is_finished());
    assert_eq!(
        Arc::strong_count(&state),
        2,
        "only the final timer may retain state"
    );
    assert_eq!(state.get(), DccState::DisableInitiation);
    tokio::task::yield_now().await;
    advance(Duration::from_secs(60)).await;
    tokio::task::yield_now().await;
    assert_eq!(state.get(), DccState::DisableInitiation);
    advance(Duration::from_secs(60)).await;
    let handle = timer.lock().await.take().unwrap();
    handle.await.unwrap();
    assert_eq!(state.get(), DccState::Enable);
}

#[tokio::test(start_paused = true)]
async fn dcc_duration_extension_none_zero_and_enable_are_preserved() {
    let state = comm_state_in(DccState::Enable);
    let timer = Arc::new(Mutex::new(crate::server::dcc_timer::TimerSlot::default()));
    dispatch(&state, &timer, EnableDisable::DISABLE_INITIATION, Some(1)).await;
    tokio::task::yield_now().await;
    advance(Duration::from_secs(30)).await;
    dispatch(&state, &timer, EnableDisable::DISABLE_INITIATION, Some(2)).await;
    tokio::task::yield_now().await;
    advance(Duration::from_secs(119)).await;
    tokio::task::yield_now().await;
    assert_eq!(state.get(), DccState::DisableInitiation);
    advance(Duration::from_secs(1)).await;
    let handle = timer.lock().await.take().unwrap();
    handle.await.unwrap();
    assert_eq!(state.get(), DccState::Enable);
    for mode in [EnableDisable::DISABLE_INITIATION, EnableDisable::ENABLE] {
        for duration in [Some(0), None, Some(1)] {
            dispatch(&state, &timer, mode, duration).await;
            assert_eq!(
                state.get(),
                if mode == EnableDisable::ENABLE {
                    DccState::Enable
                } else {
                    DccState::DisableInitiation
                }
            );
            assert_eq!(timer.lock().await.is_some(), duration.is_some());
            if let Some(minutes) = duration {
                tokio::task::yield_now().await;
                advance(Duration::from_secs(minutes as u64 * 60)).await;
                let handle = timer.lock().await.take().unwrap();
                handle.await.unwrap();
                assert_eq!(state.get(), DccState::Enable);
            } else {
                advance(Duration::from_secs(121)).await;
                assert_eq!(
                    state.get(),
                    if mode == EnableDisable::ENABLE {
                        DccState::Enable
                    } else {
                        DccState::DisableInitiation
                    }
                );
            }
        }
    }
}
