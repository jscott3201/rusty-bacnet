use super::heartbeat::*;
use super::heartbeat_test_support::*;
use super::*;
use std::sync::atomic::AtomicU16;

#[tokio::test]
async fn ack_and_timeout_linearize_in_both_map_lock_orders() {
    for ack_first in [true, false] {
        let clients = clients();
        let mut live = LiveClient::connect(clients.clone(), [0x22; 6]).await;
        live.idle().await;
        sweep(&clients, &AtomicU16::new(1), &ClockIo(AtomicU64::new(100))).await;
        assert_eq!(live.recv().await.message_id, 1);
        let attempt = snapshot(&clients, 106).await.remove(0).0;
        let clock = ClockIo(AtomicU64::new(106));
        let guard = clients.lock().await;
        let ack = clear_matching_heartbeat_ack(&clients, live.vmac, &attempt.sink, 1);
        let timeout = retire(&clients, &attempt, Retirement::AckTimeout, &clock);
        tokio::pin!(ack, timeout);
        // Queue actual production transitions behind the map lock in each order.
        if ack_first {
            assert!(futures_util::poll!(&mut ack).is_pending());
            assert!(futures_util::poll!(&mut timeout).is_pending());
        } else {
            assert!(futures_util::poll!(&mut timeout).is_pending());
            assert!(futures_util::poll!(&mut ack).is_pending());
        }
        drop(guard);
        let ((), removed) = tokio::join!(ack, timeout);
        assert_eq!(removed, !ack_first);
        if ack_first {
            let state = clients.lock().await.get(&live.vmac).unwrap().heartbeat;
            assert_eq!(state.generation, attempt.generation);
            assert!(state.pending.is_none());
        } else {
            // This timeout snapshot deliberately retains an Arc to the sink;
            // verify reader termination, not socket drop while we still own it.
            tokio::time::timeout(std::time::Duration::from_secs(2), &mut live.reader)
                .await
                .unwrap()
                .unwrap();
            assert!(!clients.lock().await.contains_key(&live.vmac));
        }
    }
}

#[tokio::test]
async fn stale_failure_and_timeout_preserve_new_pending_and_new_acked_generation() {
    let clients = clients();
    let mut live = LiveClient::connect(clients.clone(), [0x22; 6]).await;
    live.idle().await;
    sweep(&clients, &AtomicU16::new(1), &ClockIo(AtomicU64::new(100))).await;
    assert_eq!(live.recv().await.message_id, 1);
    let old = snapshot(&clients, 106).await.remove(0).0;
    // Recheck the deadline at retirement, not just the advisory snapshot.
    assert!(
        !retire(
            &clients,
            &old,
            Retirement::AckTimeout,
            &ClockIo(AtomicU64::new(105))
        )
        .await
    );
    live.ack(1).await;
    live.idle().await;
    sweep(&clients, &AtomicU16::new(2), &ClockIo(AtomicU64::new(200))).await;
    assert_eq!(live.recv().await.message_id, 2);
    for acked in [false, true] {
        if acked {
            live.ack(2).await;
        }
        let before = clients.lock().await.get(&live.vmac).unwrap().heartbeat;
        assert_eq!(before.generation, old.generation + 1);
        assert_eq!(before.pending.is_none(), acked);
        for reason in [Retirement::SendFailed, Retirement::AckTimeout] {
            assert!(!retire(&clients, &old, reason, &ClockIo(AtomicU64::new(300))).await);
            let map = clients.lock().await;
            let c = map.get(&live.vmac).unwrap();
            assert_eq!(c.heartbeat, before);
            assert!(!c.closed.load(Ordering::Acquire));
        }
    }
}

#[tokio::test]
async fn replacement_during_sink_wait_rejects_old_send_ack_and_retirement() {
    let clients = clients();
    let mut old = LiveClient::connect(clients.clone(), [0x22; 6]).await;
    old.idle().await;
    let sink = old.sink().await;
    let guard = sink.lock().await;
    let clock = ClockIo(AtomicU64::new(100));
    let ids = AtomicU16::new(1);
    let old_work = sweep(&clients, &ids, &clock);
    tokio::pin!(old_work);
    assert!(futures_util::poll!(&mut old_work).is_pending());
    let old_attempt = snapshot(&clients, 106).await.remove(0).0;
    let mut replacement = LiveClient::connect(clients.clone(), old.vmac).await;
    replacement.idle().await;
    sweep(&clients, &AtomicU16::new(1), &clock).await;
    assert_eq!(replacement.recv().await.message_id, 1);
    let before = clients.lock().await.get(&old.vmac).unwrap().heartbeat;
    assert_eq!(before.generation, old_attempt.generation); // identity, not just generation
    clear_matching_heartbeat_ack(&clients, old.vmac, &sink, 1).await;
    assert!(!retire(&clients, &old_attempt, Retirement::SendFailed, &clock).await);
    assert!(
        !retire(
            &clients,
            &old_attempt,
            Retirement::AckTimeout,
            &ClockIo(AtomicU64::new(106))
        )
        .await
    );
    drop(guard);
    old_work.await; // must recheck registration after acquiring the old sink
    assert_eq!(
        clients.lock().await.get(&old.vmac).unwrap().heartbeat,
        before
    );
    old.expect_closed().await; // no heartbeat was sent to the superseded socket
    replacement.ack(1).await;
}

#[tokio::test]
async fn advisory_reservation_rechecks_activity_pending_closed_and_identity() {
    let clients = clients();
    let mut live = LiveClient::connect(clients.clone(), [0x22; 6]).await;
    live.idle().await;
    let candidate = snapshot(&clients, 100).await.remove(0).0;
    let clock = ClockIo(AtomicU64::new(100));
    clients
        .lock()
        .await
        .get(&live.vmac)
        .unwrap()
        .last_activity
        .store(100, Ordering::Release);
    assert!(reserve(&clients, &candidate, 1, &clock).await.is_none());
    live.idle().await;
    clients
        .lock()
        .await
        .get(&live.vmac)
        .unwrap()
        .closed
        .store(true, Ordering::Release);
    assert!(reserve(&clients, &candidate, 1, &clock).await.is_none());
    clients
        .lock()
        .await
        .get(&live.vmac)
        .unwrap()
        .closed
        .store(false, Ordering::Release);
    clock.0.store(200, Ordering::Release);
    let attempt = reserve(&clients, &candidate, 0, &clock).await.unwrap();
    assert_eq!(
        clients
            .lock()
            .await
            .get(&live.vmac)
            .unwrap()
            .heartbeat
            .pending,
        Some(PendingHeartbeat {
            message_id: 0,
            published_at: 200
        })
    );
    assert!(reserve(&clients, &candidate, 1, &clock).await.is_none());
    live.ack(0).await; // zero matches like every other wire ID
    let before = clients.lock().await.get(&live.vmac).unwrap().heartbeat;
    live.ack(0).await; // unsolicited/repeated ACK changes no heartbeat state
    assert_eq!(
        clients.lock().await.get(&live.vmac).unwrap().heartbeat,
        before
    );
    assert_eq!(before.generation, attempt.generation);
    let mut replacement = LiveClient::connect(clients.clone(), live.vmac).await;
    replacement.idle().await;
    assert!(reserve(&clients, &candidate, 1, &clock).await.is_none());
    replacement
        .send(frame(ScFunction::HeartbeatRequest, 9))
        .await;
    assert_eq!(replacement.recv().await.message_id, 9);
}

#[tokio::test]
async fn exhausted_local_generation_retires_instead_of_wrapping() {
    let clients = clients();
    let mut live = LiveClient::connect(clients.clone(), [0x22; 6]).await;
    live.idle().await;
    let closed = {
        let mut map = clients.lock().await;
        let c = map.get_mut(&live.vmac).unwrap();
        c.heartbeat.generation = u64::MAX;
        c.closed.clone()
    };
    sweep(&clients, &AtomicU16::new(1), &ClockIo(AtomicU64::new(100))).await;
    assert!(!clients.lock().await.contains_key(&live.vmac));
    assert!(closed.load(Ordering::Acquire));
    live.expect_closed().await;
}
