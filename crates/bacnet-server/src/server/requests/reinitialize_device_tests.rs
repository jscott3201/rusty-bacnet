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
    dispatch_from(service_request, config, initial, None).await
}

/// The request's link source.
const SOURCE_MAC: [u8; 6] = [127, 0, 0, 1, 0xba, 0xc0];

async fn dispatch_from(
    service_request: Bytes,
    config: ServerConfig,
    initial: DccState,
    source_network: Option<NpduAddress>,
) -> Apdu {
    let restart = ReinitializeDeviceRequest::decode(&service_request).is_ok_and(|request| {
        matches!(
            request.reinitialized_state,
            ReinitializedState::WARMSTART | ReinitializedState::COLDSTART
        )
    });
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
        &SOURCE_MAC,
        source_network,
        request,
        Some(tx),
    )
    .await;
    assert!(dcc_timer.lock().await.is_none());
    let npdu = decode_npdu(rx.await.unwrap()).unwrap();
    let reply = decode_apdu(npdu.payload).unwrap();
    let expected = if restart && matches!(&reply, Apdu::SimpleAck(_)) {
        DccState::Enable
    } else {
        initial
    };
    assert_eq!(comm_state.get(), expected);
    reply
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

type Received = Arc<std::sync::Mutex<Vec<ReinitializeContext>>>;

/// A handler that records the context of each request it is asked to carry
/// out, then returns what `outcome` does.
fn recording_config(
    password: Option<&str>,
    outcome: impl Fn() -> Result<(), Error> + Send + Sync + 'static,
) -> (ServerConfig, Received) {
    let received = Received::default();
    let recorded = Arc::clone(&received);
    let handler: ReinitializeHandler = Arc::new(
        move |context: &ReinitializeContext, _database: &mut ObjectDatabase| {
            recorded.lock().unwrap().push(context.clone());
            outcome()
        },
    );
    let config = ServerConfig {
        reinit_password: password.map(str::to_owned),
        on_reinitialize: Some(handler),
        ..Default::default()
    };
    (config, received)
}

fn states(received: &Received) -> Vec<ReinitializedState> {
    received.lock().unwrap().iter().map(|c| c.state).collect()
}

fn protocol(class: ErrorClass, code: ErrorCode) -> Error {
    Error::Protocol {
        class: class.to_raw() as u32,
        code: code.to_raw() as u32,
    }
}

#[tokio::test(start_paused = true)]
async fn reinitialize_device_passes_every_defined_state_to_the_handler() {
    for state in DEFINED_STATES {
        for initial in [DccState::Enable, DccState::DisableInitiation] {
            let (config, received) = recording_config(Some("reinit-pw"), || Ok(()));

            let apdu = dispatch_with(request_data(state, Some("reinit-pw")), config, initial).await;

            let Apdu::SimpleAck(ack) = apdu else {
                panic!("expected SimpleACK, got {apdu:?}")
            };
            assert_eq!(ack.invoke_id, 42);
            assert_eq!(
                ack.service_choice,
                ConfirmedServiceChoice::REINITIALIZE_DEVICE
            );
            assert_eq!(states(&received), vec![state]);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn reinitialize_device_refuses_an_undefined_state_without_the_handler() {
    for state in UNDEFINED_STATES {
        let (config, received) = recording_config(Some("reinit-pw"), || Ok(()));

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
    let (config, _) = recording_config(None, || {
        Err(protocol(
            ErrorClass::DEVICE,
            ErrorCode::CONFIGURATION_IN_PROGRESS,
        ))
    });

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
    let (config, received) = recording_config(Some("reinit-pw"), || Ok(()));

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

/// The handler may have partly acted before it panicked, so the reply is
/// SERVICES / OTHER: never a SimpleACK, a Reject, an Abort or silence.
#[tokio::test(start_paused = true)]
async fn reinitialize_device_handler_panic_is_answered_services_other() {
    let (config, received) = recording_config(None, || panic!("reinitialize handler panic"));

    assert_error(
        dispatch_with(
            request_data(ReinitializedState::WARMSTART, None),
            config,
            DccState::Enable,
        )
        .await,
        ErrorClass::SERVICES,
        ErrorCode::OTHER,
    );
    assert_eq!(states(&received), vec![ReinitializedState::WARMSTART]);
}

/// Once the handler has run the request may no longer be rejected (Clause
/// 20.1.8), so a Reject, or any error with no class and code, is sent as
/// SERVICES / OTHER, while a structured refusal keeps its class and code.
#[tokio::test(start_paused = true)]
async fn reinitialize_device_handler_reject_is_answered_services_other() {
    let reject = || Error::Reject {
        reason: RejectReason::PARAMETER_OUT_OF_RANGE.to_raw(),
    };
    let abort = || Error::Abort {
        reason: AbortReason::OTHER.to_raw(),
    };
    let encoding = || Error::Encoding("handler failure".into());
    for error in [reject, abort, encoding] {
        let (config, _) = recording_config(None, move || Err(error()));
        assert_error(
            dispatch_with(
                request_data(ReinitializedState::START_RESTORE, None),
                config,
                DccState::Enable,
            )
            .await,
            ErrorClass::SERVICES,
            ErrorCode::OTHER,
        );
    }
    let (config, _) = recording_config(None, || {
        Err(Error::Structured {
            class: ErrorClass::DEVICE.to_raw() as u32,
            code: ErrorCode::CONFIGURATION_IN_PROGRESS.to_raw() as u32,
            detail: Box::new(bacnet_types::error::ErrorDetail::FirstFailedElementNumber(
                1,
            )),
        })
    });
    assert_error(
        dispatch_with(
            request_data(ReinitializedState::START_RESTORE, None),
            config,
            DccState::Enable,
        )
        .await,
        ErrorClass::DEVICE,
        ErrorCode::CONFIGURATION_IN_PROGRESS,
    );
}

/// The handler sees who asked, direct or routed, and the request's invoke
/// ID; its Debug output carries the address lengths, not the addresses.
#[tokio::test(start_paused = true)]
async fn reinitialize_device_context_names_the_requester() {
    // Octets that appear nowhere else in the Debug output (215 and 0xd7).
    let routed = NpduAddress {
        network: 77,
        mac_address: MacAddr::from_slice(&[0xd7, 0xd7]),
    };
    for source_network in [None, Some(routed)] {
        let (config, received) = recording_config(None, || Ok(()));
        let apdu = dispatch_from(
            request_data(ReinitializedState::ACTIVATE_CHANGES, None),
            config,
            DccState::Enable,
            source_network.clone(),
        )
        .await;
        assert!(matches!(apdu, Apdu::SimpleAck(_)), "got {apdu:?}");
        let context = received.lock().unwrap().pop().unwrap();
        assert_eq!(context.state, ReinitializedState::ACTIVATE_CHANGES);
        assert_eq!(context.source_mac.as_slice(), &SOURCE_MAC);
        assert_eq!(context.source_network, source_network);
        assert_eq!(context.invoke_id, 42);
        assert_eq!(
            context.provenance,
            bacnet_transport::port::TransportProvenance::unverified()
        );
        assert!(context.direct_sc_identity().is_none());
        assert_eq!(
            context,
            ReinitializeContext::new(
                ReinitializedState::ACTIVATE_CHANGES,
                MacAddr::from_slice(&SOURCE_MAC),
                source_network.clone(),
                bacnet_transport::port::TransportProvenance::unverified(),
                42,
            )
        );
        let debug = format!("{context:?}");
        assert!(debug.contains("source_mac_len: 6"), "{debug}");
        assert!(!debug.contains("186"), "{debug}");
        if source_network.is_some() {
            assert!(debug.contains("source_network: Some((77, 2))"), "{debug}");
        }
        assert!(
            !debug.contains("215") && !debug.to_lowercase().contains("d7"),
            "{debug}"
        );
    }
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

#[path = "reinitialize_dcc_tests.rs"]
mod dcc;
