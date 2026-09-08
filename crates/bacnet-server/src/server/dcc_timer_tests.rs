use super::*;
use bacnet_services::device_mgmt::DeviceCommunicationControlRequest;
use bacnet_types::enums::EnableDisable;

async fn activate(tx: &mpsc::Sender<ReceivedNpdu>) {
    let mut data = BytesMut::new();
    DeviceCommunicationControlRequest {
        time_duration: Some(1),
        enable_disable: EnableDisable::DISABLE_INITIATION,
        password: None,
    }
    .encode(&mut data)
    .unwrap();
    let Apdu::ConfirmedRequest(mut request) = confirmed(false) else {
        unreachable!()
    };
    request.service_choice = ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL;
    request.service_request = data.freeze();
    inject(tx, Apdu::ConfirmedRequest(request)).await;
}

#[tokio::test(start_paused = true)]
async fn dcc_timer_stop_joins_live_duration_without_reset() {
    let (mut server, tx, mut started) = fixture().await;
    activate(&tx).await;
    let mut response_resource = started.recv().await.unwrap();
    tokio::task::yield_now().await;
    let timer = server
        .dcc_timer
        .lock()
        .await
        .as_ref()
        .unwrap()
        .abort_handle();
    assert_eq!(server.comm_state(), 2);
    server.stop().await.unwrap();
    assert_eq!(response_resource.try_recv(), Ok(()));
    assert!(timer.is_finished(), "stop returned with a live DCC timer");
    assert!(server.dcc_timer.lock().await.is_none());
    assert_eq!(server.comm_state(), 2);
    tokio::time::advance(Duration::from_secs(61)).await;
    tokio::task::yield_now().await;
    assert_eq!(server.comm_state(), 2);
    server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn dcc_timer_cancelled_stop_retains_join_for_retry() {
    let (mut server, _tx, _started) = fixture().await;
    // Remove dispatch as an earlier suspension point, so the single poll below
    // reaches the timer's pending join without scheduling its destruction.
    let dispatch = server.dispatch_task.take().unwrap();
    dispatch.abort();
    let _ = dispatch.await;
    let (released, mut resource) = oneshot::channel();
    let guard = SendGuard(Some(released));
    let task = tokio::spawn(async move {
        let _guard = guard;
        std::future::pending::<()>().await;
    });
    let id = task.id();
    *server.dcc_timer.lock().await = Some(task);
    server.comm_state.store(2, Ordering::Release);
    {
        let stop = server.stop();
        tokio::pin!(stop);
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(stop.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
    }
    assert_eq!(server.dcc_timer.lock().await.as_ref().unwrap().id(), id);
    assert_eq!(
        resource.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    );
    assert_eq!(server.comm_state(), 2);
    server.stop().await.unwrap();
    assert_eq!(
        resource.try_recv(),
        Ok(()),
        "cleanup must precede stop return"
    );
    assert!(server.dcc_timer.lock().await.is_none());
    assert_eq!(server.comm_state(), 2);
    server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn dcc_timer_stop_handles_absent_and_completed_timer() {
    let (mut server, tx, mut started) = fixture().await;
    assert!(server.dcc_timer.lock().await.is_none());
    activate(&tx).await;
    let _response_resource = started.recv().await.unwrap();
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(60)).await;
    tokio::task::yield_now().await;
    assert!(server
        .dcc_timer
        .lock()
        .await
        .as_ref()
        .unwrap()
        .is_finished());
    assert_eq!(server.comm_state(), 0);
    server.stop().await.unwrap();
    assert!(server.dcc_timer.lock().await.is_none());
    server.stop().await.unwrap();
    let (mut empty, _tx, _started) = fixture().await;
    empty.stop().await.unwrap();
    assert!(empty.dcc_timer.lock().await.is_none());
}
