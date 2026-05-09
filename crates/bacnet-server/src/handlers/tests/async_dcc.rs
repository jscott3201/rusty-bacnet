use super::*;

// -----------------------------------------------------------------------
// DCC timer auto-re-enable tests (Clause 16.4.3)
// -----------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn dcc_timer_auto_re_enables() {
    use std::sync::Arc;

    let comm_state = Arc::new(AtomicU8::new(0));

    // Send DCC DISABLE with 1-minute duration
    let request = bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
        time_duration: Some(1), // 1 minute
        enable_disable: EnableDisable::DISABLE_INITIATION,
        password: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();

    let (state, duration) = handle_device_communication_control(&buf, &comm_state, &None).unwrap();
    assert_eq!(state, EnableDisable::DISABLE_INITIATION);
    assert_eq!(duration, Some(1));
    assert_eq!(comm_state.load(Ordering::Acquire), 2);

    // Simulate what the server dispatch does: spawn a timer task
    let comm_clone = Arc::clone(&comm_state);
    let minutes = duration.unwrap();
    let handle = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(minutes as u64 * 60)).await;
        comm_clone.store(0, Ordering::Release);
    });

    // State should still be DISABLE before timer fires
    assert_eq!(comm_state.load(Ordering::Acquire), 2);

    // Advance time past the 1-minute duration
    tokio::time::advance(std::time::Duration::from_secs(61)).await;
    // Wait for the spawned task to complete (which sets state back to 0)
    handle.await.unwrap();

    // State should now be re-enabled
    assert_eq!(comm_state.load(Ordering::Acquire), 0);
}

#[tokio::test(start_paused = true)]
async fn dcc_timer_cancelled_by_new_dcc() {
    use std::sync::Arc;
    use tokio::task::JoinHandle;

    let comm_state = Arc::new(AtomicU8::new(0));

    // Send DCC DISABLE with 2-minute duration
    let request = bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
        time_duration: Some(2),
        enable_disable: EnableDisable::DISABLE_INITIATION,
        password: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();
    let (_, duration) = handle_device_communication_control(&buf, &comm_state, &None).unwrap();
    assert_eq!(comm_state.load(Ordering::Acquire), 2);

    // Spawn first timer
    let comm_clone = Arc::clone(&comm_state);
    let minutes = duration.unwrap();
    let handle1: JoinHandle<()> = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(minutes as u64 * 60)).await;
        comm_clone.store(0, Ordering::Release);
    });

    // Now send DCC ENABLE (no duration) — should cancel the timer
    let request2 = bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
        time_duration: None,
        enable_disable: EnableDisable::ENABLE,
        password: None,
    };
    let mut buf2 = BytesMut::new();
    request2.encode(&mut buf2).unwrap();
    let (state2, duration2) =
        handle_device_communication_control(&buf2, &comm_state, &None).unwrap();
    assert_eq!(state2, EnableDisable::ENABLE);
    assert_eq!(duration2, None);
    assert_eq!(comm_state.load(Ordering::Acquire), 0);

    // Abort previous timer (simulating server dispatch behavior)
    handle1.abort();

    // Advance past the original 2-minute duration
    tokio::time::advance(std::time::Duration::from_secs(121)).await;
    tokio::task::yield_now().await;

    // State should still be ENABLE (timer was cancelled)
    assert_eq!(comm_state.load(Ordering::Acquire), 0);
}
