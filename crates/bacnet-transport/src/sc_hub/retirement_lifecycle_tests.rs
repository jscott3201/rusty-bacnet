use super::deadline_capacity_tests::CountedHub;
use super::deadline_test_support::*;
use super::heartbeat_test_support::ClockIo;
use super::retirement_tests::{heartbeat_wire, unicast_wire};
use super::shutdown_blocked_tests::ControlledPeer;
use super::*;
use std::time::Duration;

#[tokio::test]
async fn replacement_interrupts_accept_ack_and_source_relay_preserving_new_identity() {
    for phase in 0..3 {
        let tls = TestTls::new();
        let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
        let mut old = ControlledPeer::open(&tls, &hub).await;
        let mut target = ControlledPeer::open(&tls, &hub).await;
        target.connect(0x43).await;
        if phase != 0 {
            old.connect(0x42).await;
        }
        let held = if phase == 2 {
            target.sink.clone()
        } else {
            old.sink.clone()
        }
        .lock_owned()
        .await;
        if phase == 0 {
            old.ws.send(request([0x42; 6], [0x42; 16])).await.unwrap();
            until(|| old.deadline.is_committed()).await;
        } else {
            old.ws
                .send(if phase == 1 {
                    heartbeat_wire(23)
                } else {
                    unicast_wire([0x43; 6])
                })
                .await
                .unwrap();
            until(|| old.deadline.received.load(Ordering::Acquire) == 2).await;
        }
        finish_replacement(&mut hub, &tls, &mut old, held).await;
    }
}

async fn finish_replacement(
    hub: &mut CountedHub,
    tls: &TestTls,
    old: &mut ControlledPeer,
    held: tokio::sync::OwnedMutexGuard<WsSink>,
) {
    let mut replacement = ControlledPeer::open(tls, hub).await;
    // Same VMAC and UUID makes cleanup's sink-identity check essential.
    replacement.connect(0x42).await;
    until(|| old.deadline.close_started.load(Ordering::Acquire)).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(6)).await;
    until(|| hub.active.load(Ordering::Acquire) == 2).await; // replacement + target
    let map = hub.clients.lock().await;
    let current = map.get(&[0x42; 6]).unwrap();
    assert!(Arc::ptr_eq(&current.sink, &replacement.sink));
    assert!(!current.closed.load(Ordering::Acquire));
    drop(map);
    replacement.ws.send(heartbeat_wire(25)).await.unwrap();
    assert!(matches!(poll_io(replacement.ws.next()).await,
        Some(Ok(Message::Binary(data))) if data[0..4] == [0x0B, 0, 0, 25]));
    drop(held);
    hub.hub.stop().await;
    tokio::time::resume();
}

#[tokio::test]
async fn retirement_releases_multiple_target_waiters_without_retiring_sources() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut target = ControlledPeer::open(&tls, &hub).await;
    target.connect(0x43).await;
    let held = target.sink.clone().lock_owned().await;
    let mut sources = Vec::new();
    for id in [0x42, 0x44] {
        let mut source = ControlledPeer::open(&tls, &hub).await;
        source.connect(id).await;
        source.ws.send(unicast_wire([0x43; 6])).await.unwrap();
        until(|| source.deadline.received.load(Ordering::Acquire) == 2).await;
        sources.push(source);
    }
    let attempt = heartbeat::Attempt {
        vmac: [0x43; 6],
        sink: target.sink.clone(),
        generation: 0,
    };
    assert!(
        heartbeat::retire(
            &hub.clients,
            &attempt,
            heartbeat::Retirement::SendFailed,
            &ClockIo(AtomicU64::new(106))
        )
        .await
    );
    for source in &mut sources {
        source.ws.send(heartbeat_wire(25)).await.unwrap();
        assert!(matches!(poll_io(source.ws.next()).await,
            Some(Ok(Message::Binary(data))) if data[0..4] == [0x0B, 0, 0, 25]));
    }
    assert_eq!(hub.clients.lock().await.len(), 2);
    drop(held);
    until(|| hub.active.load(Ordering::Acquire) == 2).await;
    hub.hub.stop().await;
}

#[tokio::test]
async fn retirement_waiters_ignore_spurious_wakes_and_observe_retained_closed_state() {
    let closed = Arc::new(AtomicBool::new(false));
    let notify = Arc::new(Notify::new());
    let mut waiters = Vec::new();
    for _ in 0..3 {
        let closed = closed.clone();
        let notify = notify.clone();
        let (ready, started) = tokio::sync::oneshot::channel();
        let waiter = tokio::spawn(async move {
            let future = retirement::wait(&closed, &notify);
            tokio::pin!(future);
            assert!(futures_util::poll!(&mut future).is_pending());
            ready.send(()).unwrap();
            future.await;
        });
        started.await.unwrap();
        waiters.push(waiter);
    }
    retirement::wake(&notify);
    tokio::task::yield_now().await;
    assert!(waiters.iter().all(|waiter| !waiter.is_finished()));
    closed.store(true, Ordering::Release);
    retirement::wake(&notify);
    for waiter in waiters {
        poll_io(waiter).await.unwrap();
    }
    // Consume any retained notification; late waiters still see the predicate.
    let _ = futures_util::FutureExt::now_or_never(notify.notified());
    assert!(futures_util::FutureExt::now_or_never(retirement::wait(&closed, &notify)).is_some());
}
