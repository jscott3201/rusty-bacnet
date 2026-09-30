use super::*;

use bacnet_encoding::{apdu::decode_apdu, npdu::decode_npdu};
use bacnet_services::device_mgmt::ReinitializeDeviceRequest;
use bacnet_types::enums::ReinitializedState;

const DEFINED_STATES: [ReinitializedState; 8] = [
    ReinitializedState::COLDSTART,
    ReinitializedState::WARMSTART,
    ReinitializedState::START_BACKUP,
    ReinitializedState::END_BACKUP,
    ReinitializedState::START_RESTORE,
    ReinitializedState::END_RESTORE,
    ReinitializedState::ABORT_RESTORE,
    ReinitializedState::ACTIVATE_CHANGES,
];

/// States Clause 16.4 does not define, which the decoder keeps.
const UNDEFINED_STATES: [ReinitializedState; 2] = [
    ReinitializedState::from_raw(8),
    ReinitializedState::from_raw(u32::MAX),
];

fn every_state() -> impl Iterator<Item = ReinitializedState> {
    DEFINED_STATES.into_iter().chain(UNDEFINED_STATES)
}

fn request_data(state: ReinitializedState, password: Option<&str>) -> Bytes {
    let mut data = BytesMut::new();
    ReinitializeDeviceRequest {
        reinitialized_state: state,
        password: password.map(str::to_owned),
    }
    .encode(&mut data)
    .unwrap();
    data.freeze()
}

async fn dispatch(service_request: Bytes, password: Option<&str>, initial: DccState) -> Apdu {
    let config = ServerConfig {
        reinit_password: password.map(str::to_owned),
        ..Default::default()
    };
    dispatch_with(service_request, config, initial).await
}

async fn dispatch_with(service_request: Bytes, config: ServerConfig, initial: DccState) -> Apdu {
    let network = Arc::new(NetworkLayer::new(BipTransport::new(
        Ipv4Addr::LOCALHOST,
        0,
        Ipv4Addr::BROADCAST,
    )));
    let comm_state = Arc::new(CommState::default());
    comm_state.set_for_test(initial);
    let dcc_timer = Arc::new(Mutex::new(crate::server::dcc_timer::TimerSlot::default()));
    let request = ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 480,
        invoke_id: 42,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::REINITIALIZE_DEVICE,
        service_request,
    };
    let (tx, rx) = oneshot::channel();
    BACnetServer::<BipTransport>::handle_confirmed_request(
        &RequestServices {
            comm_state: Arc::clone(&comm_state),
            dcc_timer: Arc::clone(&dcc_timer),
            ..RequestServices::for_test(Arc::clone(&network), config.clone())
        },
        &Arc::new(ConfirmedRequestTracker::default()),
        &Arc::new(crate::server::request_tasks::RequestTasks::default()).spawner(),
        &[127, 0, 0, 1, 0xba, 0xc0],
        None,
        request,
        Some(tx),
    )
    .await;
    assert_eq!(comm_state.get(), initial);
    assert!(dcc_timer.lock().await.is_none());
    let npdu = decode_npdu(rx.await.unwrap()).unwrap();
    decode_apdu(npdu.payload).unwrap()
}

fn assert_error(apdu: Apdu, class: ErrorClass, code: ErrorCode) {
    let Apdu::Error(error) = apdu else {
        panic!("expected Error, never SimpleACK, got {apdu:?}")
    };
    assert_eq!(error.invoke_id, 42);
    assert_eq!(
        error.service_choice,
        ConfirmedServiceChoice::REINITIALIZE_DEVICE
    );
    assert_eq!(error.error_class, class);
    assert_eq!(error.error_code, code);
    assert!(error.error_data.is_empty());
}

#[tokio::test(start_paused = true)]
async fn reinitialize_device_refuses_all_states_after_password_validation() {
    for state in every_state() {
        for (configured, supplied) in [
            (Some("reinit-pw"), Some("reinit-pw")),
            (None, None),
            (None, Some("anything")),
        ] {
            for initial in [DccState::Enable, DccState::DisableInitiation] {
                assert_error(
                    dispatch(request_data(state, supplied), configured, initial).await,
                    ErrorClass::SERVICES,
                    ErrorCode::SERVICE_REQUEST_DENIED,
                );
            }
        }
    }
}

#[tokio::test(start_paused = true)]
async fn reinitialize_device_password_failure_precedes_refusal() {
    for state in every_state() {
        for supplied in [None, Some("wrong")] {
            for initial in [DccState::Enable, DccState::DisableInitiation] {
                assert_error(
                    dispatch(request_data(state, supplied), Some("reinit-pw"), initial).await,
                    ErrorClass::SECURITY,
                    ErrorCode::PASSWORD_FAILURE,
                );
            }
        }
    }
}

/// A handler that records each state it is asked for, and refuses with `refusal` if given.
fn recording_config(
    password: Option<&str>,
    refusal: Option<(ErrorClass, ErrorCode)>,
) -> (ServerConfig, Arc<std::sync::Mutex<Vec<ReinitializedState>>>) {
    let received = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = Arc::clone(&received);
    let handler: ReinitializeHandler = Arc::new(move |state, _database: &mut ObjectDatabase| {
        recorded.lock().unwrap().push(state);
        match refusal {
            None => Ok(()),
            Some((class, code)) => Err(Error::Protocol {
                class: class.to_raw() as u32,
                code: code.to_raw() as u32,
            }),
        }
    });
    let config = ServerConfig {
        reinit_password: password.map(str::to_owned),
        on_reinitialize: Some(handler),
        ..Default::default()
    };
    (config, received)
}

#[tokio::test(start_paused = true)]
async fn reinitialize_device_passes_every_defined_state_to_the_handler() {
    for state in DEFINED_STATES {
        for initial in [DccState::Enable, DccState::DisableInitiation] {
            let (config, received) = recording_config(Some("reinit-pw"), None);

            let apdu = dispatch_with(request_data(state, Some("reinit-pw")), config, initial).await;

            let Apdu::SimpleAck(ack) = apdu else {
                panic!("expected SimpleACK, got {apdu:?}")
            };
            assert_eq!(ack.invoke_id, 42);
            assert_eq!(
                ack.service_choice,
                ConfirmedServiceChoice::REINITIALIZE_DEVICE
            );
            assert_eq!(*received.lock().unwrap(), vec![state]);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn reinitialize_device_refuses_an_undefined_state_without_the_handler() {
    for state in UNDEFINED_STATES {
        let (config, received) = recording_config(Some("reinit-pw"), None);

        assert_error(
            dispatch_with(
                request_data(state, Some("reinit-pw")),
                config,
                DccState::Enable,
            )
            .await,
            ErrorClass::SERVICES,
            ErrorCode::SERVICE_REQUEST_DENIED,
        );
        assert!(received.lock().unwrap().is_empty());
    }
}

#[tokio::test(start_paused = true)]
async fn reinitialize_device_sends_the_handlers_error() {
    let (config, _) = recording_config(
        None,
        Some((ErrorClass::DEVICE, ErrorCode::CONFIGURATION_IN_PROGRESS)),
    );

    assert_error(
        dispatch_with(
            request_data(ReinitializedState::START_BACKUP, None),
            config,
            DccState::Enable,
        )
        .await,
        ErrorClass::DEVICE,
        ErrorCode::CONFIGURATION_IN_PROGRESS,
    );
}

#[tokio::test(start_paused = true)]
async fn reinitialize_device_password_failure_never_reaches_the_handler() {
    let (config, received) = recording_config(Some("reinit-pw"), None);

    assert_error(
        dispatch_with(
            request_data(ReinitializedState::START_BACKUP, Some("wrong")),
            config,
            DccState::Enable,
        )
        .await,
        ErrorClass::SECURITY,
        ErrorCode::PASSWORD_FAILURE,
    );
    assert!(received.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn reinitialize_device_malformed_request_precedes_password_and_refusal() {
    for data in [
        &[][..],                    // Missing state.
        &[0x09][..],                // Truncated state.
        &[0x19, 0x00][..],          // The password where the state is due.
        &[0x09, 0x01, 0x1a, 0][..], // Truncated password.
    ] {
        assert!(ReinitializeDeviceRequest::decode(data).is_err());
        for configured in [None, Some("reinit-pw")] {
            for initial in [DccState::Enable, DccState::DisableInitiation] {
                // Each is missing a parameter, which the server rejects
                // (#1446).
                let response = dispatch(Bytes::copy_from_slice(data), configured, initial).await;
                let Apdu::Reject(reject) = response else {
                    panic!("expected a Reject, got {response:?}")
                };
                assert_eq!(
                    (reject.invoke_id, reject.reject_reason),
                    (42, RejectReason::MISSING_REQUIRED_PARAMETER),
                    "{data:02X?}"
                );
            }
        }
    }
}
