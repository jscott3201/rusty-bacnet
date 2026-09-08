use super::*;

use bacnet_encoding::{apdu::decode_apdu, npdu::decode_npdu};
use bacnet_services::device_mgmt::DeviceCommunicationControlRequest;
use bacnet_types::enums::EnableDisable;
use tokio::time::{advance, Duration};

async fn dispatch(
    comm_state: &Arc<AtomicU8>,
    dcc_timer: &Arc<Mutex<Option<JoinHandle<()>>>>,
    mode: EnableDisable,
    duration: Option<u16>,
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
        password: None,
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
        comm_state,
        dcc_timer,
        &ServerConfig::default(),
        &Arc::new(crate::server::request_tasks::RequestTasks::default()).spawner(),
        &[127, 0, 0, 1, 0xba, 0xc0],
        None,
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
async fn dcc_disable_does_not_replace_cancel_or_extend_active_timer() {
    for rejected_duration in [None, Some(0), Some(1), Some(5)] {
        let state = Arc::new(AtomicU8::new(0));
        let timer = Arc::new(Mutex::new(None));
        assert!(matches!(
            dispatch(&state, &timer, EnableDisable::DISABLE_INITIATION, Some(1)).await,
            Apdu::SimpleAck(_)
        ));
        tokio::task::yield_now().await;
        let timer_id = timer.lock().await.as_ref().unwrap().id();
        advance(Duration::from_secs(30)).await;
        assert_denied(dispatch(&state, &timer, EnableDisable::DISABLE, rejected_duration).await);
        assert_eq!(state.load(Ordering::Acquire), 2);
        assert_eq!(timer.lock().await.as_ref().unwrap().id(), timer_id);
        advance(Duration::from_secs(29)).await;
        tokio::task::yield_now().await;
        assert_eq!(state.load(Ordering::Acquire), 2);
        advance(Duration::from_secs(1)).await;
        timer.lock().await.take().unwrap().await.unwrap();
        assert_eq!(state.load(Ordering::Acquire), 0);
    }
}

#[tokio::test(start_paused = true)]
async fn dcc_disable_does_not_create_timer_in_any_state() {
    for initial in [0, 1, 2] {
        let state = Arc::new(AtomicU8::new(initial));
        let timer = Arc::new(Mutex::new(None));
        assert_denied(dispatch(&state, &timer, EnableDisable::DISABLE, Some(1)).await);
        assert!(timer.lock().await.is_none());
        advance(Duration::from_secs(61)).await;
        assert_eq!(state.load(Ordering::Acquire), initial);
    }
}
