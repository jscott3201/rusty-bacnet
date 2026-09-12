use super::*;

use bacnet_encoding::{apdu::decode_apdu, npdu::decode_npdu};
use bacnet_services::device_mgmt::ReinitializeDeviceRequest;
use bacnet_types::enums::ReinitializedState;

const STATES: [ReinitializedState; 10] = [
    ReinitializedState::COLDSTART,
    ReinitializedState::WARMSTART,
    ReinitializedState::START_BACKUP,
    ReinitializedState::END_BACKUP,
    ReinitializedState::START_RESTORE,
    ReinitializedState::END_RESTORE,
    ReinitializedState::ABORT_RESTORE,
    ReinitializedState::ACTIVATE_CHANGES,
    // The decoder preserves unknown values; they must not bypass refusal either.
    ReinitializedState::from_raw(8),
    ReinitializedState::from_raw(u32::MAX),
];

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

async fn dispatch(service_request: Bytes, password: Option<&str>, initial: u8) -> Apdu {
    let network = Arc::new(NetworkLayer::new(BipTransport::new(
        Ipv4Addr::LOCALHOST,
        0,
        Ipv4Addr::BROADCAST,
    )));
    let comm_state = Arc::new(AtomicU8::new(initial));
    let dcc_timer = Arc::new(Mutex::new(None));
    let config = ServerConfig {
        reinit_password: password.map(str::to_owned),
        ..Default::default()
    };
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
        &Arc::new(RwLock::new(ObjectDatabase::new())),
        &network,
        &Arc::new(RwLock::new(CovSubscriptionTable::new())),
        &Arc::new(segmented_send::SegmentedSendRegistry::default()),
        &Arc::new(Semaphore::new(MAX_SEG_SENDERS)),
        &Arc::new(Semaphore::new(1)),
        &Arc::new(Mutex::new(ServerTsm::new())),
        &NotificationTransactions::new(),
        &Arc::new(ConfirmedRequestTracker::default()),
        &Arc::new(RwLock::new(DeviceBindingTable::new())),
        &comm_state,
        &dcc_timer,
        &config,
        &Arc::new(crate::server::request_tasks::RequestTasks::default()).spawner(),
        &[127, 0, 0, 1, 0xba, 0xc0],
        None,
        request,
        Some(tx),
    )
    .await;
    assert_eq!(comm_state.load(Ordering::Acquire), initial);
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
    for state in STATES {
        for (configured, supplied) in [
            (Some("reinit-pw"), Some("reinit-pw")),
            (None, None),
            (None, Some("anything")),
        ] {
            for initial in [0, 1, 2] {
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
    for state in STATES {
        for supplied in [None, Some("wrong")] {
            for initial in [0, 1, 2] {
                assert_error(
                    dispatch(request_data(state, supplied), Some("reinit-pw"), initial).await,
                    ErrorClass::SECURITY,
                    ErrorCode::PASSWORD_FAILURE,
                );
            }
        }
    }
}

#[tokio::test(start_paused = true)]
async fn reinitialize_device_malformed_request_precedes_password_and_refusal() {
    for data in [
        &[][..],                    // Missing state.
        &[0x09][..],                // Truncated state.
        &[0x19, 0x00][..],          // Wrong state tag.
        &[0x09, 0x01, 0x1a, 0][..], // Truncated password.
    ] {
        assert!(ReinitializeDeviceRequest::decode(data).is_err());
        for configured in [None, Some("reinit-pw")] {
            for initial in [0, 1, 2] {
                // Preserve the server's existing decode-error mapping.
                assert_error(
                    dispatch(Bytes::copy_from_slice(data), configured, initial).await,
                    ErrorClass::SERVICES,
                    ErrorCode::OTHER,
                );
            }
        }
    }
}
