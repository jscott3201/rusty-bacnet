use super::heartbeat::*;
use super::heartbeat_test_support::*;
use super::*;
use std::sync::atomic::AtomicU16;

#[tokio::test]
async fn wire_ack_during_send_completion_survives_next_sweep() {
    let clients = clients();
    let mut live = LiveClient::connect(clients.clone(), [0x22; 6]).await;
    let mut peer = LiveClient::connect(clients.clone(), [0x23; 6]).await;
    live.idle().await;
    let io = GatedIo::new(100);
    let sweep_task = tokio::spawn({
        let clients = clients.clone();
        let io = io.clone();
        async move { sweep(&clients, &AtomicU16::new(0x8000), &*io).await }
    });
    let request = live.recv().await;
    assert_eq!(request.function, ScFunction::HeartbeatRequest);
    live.ack(request.message_id).await; // production ACK dispatch has returned
                                        // Neither the sink wait nor send completion owns the clients map.
    peer.send(frame(ScFunction::HeartbeatRequest, 43)).await;
    assert_eq!(peer.recv().await.message_id, 43);
    io.release.notify_one();
    sweep_task.await.unwrap();
    sweep(
        &clients,
        &AtomicU16::new(0x8001),
        &ClockIo(AtomicU64::new(106)),
    )
    .await;
    assert!(
        clients.lock().await.contains_key(&live.vmac),
        "ACKed client was removed by next sweep"
    );
    live.send(frame(ScFunction::HeartbeatRequest, 42)).await;
    assert_eq!(live.recv().await.message_id, 42);
}

#[tokio::test]
async fn sink_wait_budget_retires_closes_and_saves_notification() {
    use futures_util::FutureExt;
    let clients = clients();
    let mut live = LiveClient::connect(clients.clone(), [0x22; 6]).await;
    live.idle().await;
    let (closed, notify) = {
        let map = clients.lock().await;
        let c = map.get(&live.vmac).unwrap();
        (c.closed.clone(), c.close_notify.clone())
    };
    // Remove the waiter to verify heartbeat retirement saves a notification.
    live.reader.abort();
    let _ = (&mut live.reader).await;
    let sink = live.sink().await;
    let guard = sink.lock().await;
    let ids = AtomicU16::new(1);
    let clock = ClockIo(AtomicU64::new(100));
    tokio::time::pause();
    let work = sweep(&clients, &ids, &clock);
    tokio::pin!(work);
    assert!(futures_util::poll!(&mut work).is_pending());
    tokio::time::advance(std::time::Duration::from_secs(4)).await;
    assert!(futures_util::poll!(&mut work).is_pending());
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    work.await;
    tokio::time::resume();
    assert!(closed.load(Ordering::Acquire));
    assert!(!clients.lock().await.contains_key(&live.vmac));
    assert!(notify.notified().now_or_never().is_some());
    drop(guard);
    drop(sink);
    live.expect_closed().await;
}

#[tokio::test]
async fn send_completion_budget_retires_and_wakes_live_reader() {
    let clients = clients();
    let mut live = LiveClient::connect(clients.clone(), [0x22; 6]).await;
    live.idle().await;
    let closed = clients.lock().await.get(&live.vmac).unwrap().closed.clone();
    let io = GatedIo::new(100);
    let task = tokio::spawn({
        let clients = clients.clone();
        let io = io.clone();
        async move { sweep(&clients, &AtomicU16::new(1), &*io).await }
    });
    assert_eq!(live.recv().await.message_id, 1);
    io.sent.notified().await;
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(5)).await;
    task.await.unwrap();
    tokio::time::resume();
    assert!(closed.load(Ordering::Acquire));
    assert!(!clients.lock().await.contains_key(&live.vmac));
    live.expect_closed().await;
}

#[tokio::test]
async fn wrapped_zero_request_still_times_out_after_wrong_wire_ack() {
    let clients = clients();
    let mut live = LiveClient::connect(clients.clone(), [0x22; 6]).await;
    let ids = AtomicU16::new(u16::MAX);
    live.idle().await;
    sweep(&clients, &ids, &ClockIo(AtomicU64::new(100))).await;
    assert_eq!(live.recv().await.message_id, u16::MAX);
    live.ack(u16::MAX).await;
    live.idle().await;
    sweep(&clients, &ids, &ClockIo(AtomicU64::new(200))).await;
    assert_eq!(live.recv().await.message_id, 0);
    live.ack(u16::MAX).await; // stale ID must not clear or extend zero's deadline
    sweep(&clients, &ids, &ClockIo(AtomicU64::new(205))).await;
    assert!(clients.lock().await.contains_key(&live.vmac)); // strict > five seconds
    sweep(&clients, &ids, &ClockIo(AtomicU64::new(206))).await;
    assert!(
        !clients.lock().await.contains_key(&live.vmac),
        "zero-ID heartbeat never expired"
    );
    live.expect_closed().await;
}

#[tokio::test]
async fn send_failure_after_matching_ack_retires_and_wakes_reader() {
    let clients = clients();
    let mut live = LiveClient::connect(clients.clone(), [0x22; 6]).await;
    let closed = clients.lock().await.get(&live.vmac).unwrap().closed.clone();
    live.idle().await;
    let io = GatedIo::new(100);
    let sweep_task = tokio::spawn({
        let clients = clients.clone();
        let io = io.clone();
        async move { sweep(&clients, &AtomicU16::new(0x8000), &*io).await }
    });
    let request = live.recv().await;
    live.ack(request.message_id).await;
    io.fail.store(true, Ordering::Release);
    io.release.notify_one();
    sweep_task.await.unwrap();
    assert!(!clients.lock().await.contains_key(&live.vmac));
    assert!(
        closed.load(Ordering::Acquire),
        "send-failure removal left reader open"
    );
    live.expect_closed().await;
}

#[tokio::test]
async fn stale_send_failure_does_not_remove_newly_reserved_attempt() {
    let clients = clients();
    let mut live = LiveClient::connect(clients.clone(), [0x22; 6]).await;
    live.idle().await;
    let io = GatedIo::new(100);
    let old_task = tokio::spawn({
        let clients = clients.clone();
        let io = io.clone();
        async move { sweep(&clients, &AtomicU16::new(1), &*io).await }
    });
    assert_eq!(live.recv().await.message_id, 1);
    live.ack(1).await;
    live.idle().await;
    let next_ids = AtomicU16::new(2);
    let clock = ClockIo(AtomicU64::new(101));
    let new_sweep = sweep(&clients, &next_ids, &clock);
    tokio::pin!(new_sweep);
    assert!(futures_util::poll!(&mut new_sweep).is_pending()); // reserved; waiting for old sink owner
    io.fail.store(true, Ordering::Release);
    io.release.notify_one();
    old_task.await.unwrap();
    assert!(
        clients.lock().await.contains_key(&live.vmac),
        "stale send failure removed newer attempt"
    );
    new_sweep.await;
    assert_eq!(live.recv().await.message_id, 2);
    live.ack(2).await;
}

#[tokio::test]
async fn closed_before_next_reader_wait_does_not_require_another_notification() {
    use futures_util::FutureExt;
    let clients = clients();
    let mut live = LiveClient::connect(clients.clone(), [0x22; 6]).await;
    let (closed, notify) = {
        let map = clients.lock().await;
        let client = map.get(&live.vmac).unwrap();
        (client.closed.clone(), client.close_notify.clone())
    };
    *live.after_ack.lock().unwrap() = Some(Box::new(move || {
        closed.store(true, Ordering::Release);
        notify.notify_one();
        // A previously selected/cancelled waiter may already have consumed the
        // permit. The owning reader must still observe the closed predicate.
        assert!(notify.notified().now_or_never().is_some());
    }));
    live.ack(7).await;
    tokio::time::timeout(std::time::Duration::from_secs(2), &mut live.reader)
        .await
        .expect("closed reader waited for a second notification")
        .unwrap();
    assert!(!clients.lock().await.contains_key(&live.vmac));
    live.expect_closed().await;
}

#[tokio::test]
async fn expired_socket_is_released_before_another_clients_blocked_send() {
    let clients = clients();
    let mut expired = LiveClient::connect(clients.clone(), [0x22; 6]).await;
    expired.idle().await;
    sweep(&clients, &AtomicU16::new(1), &ClockIo(AtomicU64::new(100))).await;
    assert_eq!(expired.recv().await.message_id, 1);
    let mut idle = LiveClient::connect(clients.clone(), [0x23; 6]).await;
    idle.idle().await;
    let io = GatedIo::new(106);
    let task = tokio::spawn({
        let clients = clients.clone();
        let io = io.clone();
        async move { sweep(&clients, &AtomicU16::new(2), &*io).await }
    });
    assert_eq!(idle.recv().await.message_id, 2);
    expired.expect_closed().await;
    assert!(!task.is_finished());
    io.release.notify_one();
    task.await.unwrap();
}
