//! Existing supervised registered retirement interrupts the new NAK, over mTLS.
use super::deadline_capacity_tests::CountedHub;
use super::deadline_test_support::*;
use super::heartbeat_test_support::ClockIo;
use super::response_silence_tests::Snapshot;
use super::shutdown_blocked_tests::ControlledPeer;
use super::*;
use std::sync::atomic::AtomicU16;
use std::time::Duration;

#[tokio::test]
async fn empty_npdu_hub_held_nak_retirement_and_replacement_preserve_other_owners() {
    for replace in [false, true] {
        let tls = TestTls::new();
        let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
        let mut old = ControlledPeer::open(&tls, &hub).await;
        old.connect(0x42).await;
        let mut healthy = ControlledPeer::open(&tls, &hub).await;
        healthy.connect(0x43).await;
        hub.clients
            .lock()
            .await
            .get(&[0x42; 6])
            .unwrap()
            .last_activity
            .store(0, Ordering::Release);
        heartbeat::sweep(
            &hub.clients,
            &AtomicU16::new(0x2233),
            &ClockIo(AtomicU64::new(100)),
        )
        .await;
        assert!(matches!(poll_io(old.ws.next()).await,
            Some(Ok(Message::Binary(data))) if data.as_ref() == [0x0A, 0, 0x22, 0x33]));
        let before = Snapshot::capture(hub.clients.lock().await.get(&[0x42; 6]).unwrap());
        let held = old.sink.clone().lock_owned().await;
        let mut empty = vec![1, 4, 0x22, 0x33];
        empty.extend_from_slice(&[0x43; 6]);
        old.ws.send(Message::Binary(empty.into())).await.unwrap();
        until(|| old.deadline.received.load(Ordering::Acquire) == 2).await;
        before.unchanged(hub.clients.lock().await.get(&[0x42; 6]).unwrap());
        // Current-thread dispatch has reached the real write-lock await. Queue
        // another frame: it must remain unread until retirement drops dispatch.
        old.ws
            .send(Message::Binary(vec![0x0A, 0, 0, 25].into()))
            .await
            .unwrap();
        let mut replacement = if replace {
            let mut peer = ControlledPeer::open(&tls, &hub).await;
            peer.connect(0x42).await;
            Some(peer)
        } else {
            heartbeat::sweep(
                &hub.clients,
                &AtomicU16::new(0x3344),
                &ClockIo(AtomicU64::new(106)),
            )
            .await;
            None
        };
        until(|| old.deadline.close_started.load(Ordering::Acquire)).await;
        assert_eq!(old.deadline.received.load(Ordering::Acquire), 2);
        // This is the pre-existing cleanup close budget, not a NAK deadline.
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(6)).await;
        until(|| hub.active.load(Ordering::Acquire) == if replace { 2 } else { 1 }).await;
        if let Some(replacement) = &mut replacement {
            let map = hub.clients.lock().await;
            assert!(Arc::ptr_eq(
                &map.get(&[0x42; 6]).unwrap().sink,
                &replacement.sink
            ));
        } else {
            assert!(!hub.clients.lock().await.contains_key(&[0x42; 6]));
        }
        for peer in std::iter::once(&mut healthy).chain(replacement.iter_mut()) {
            peer.ws
                .send(Message::Binary(vec![0x0A, 0, 0, 26].into()))
                .await
                .unwrap();
            assert!(matches!(poll_io(peer.ws.next()).await,
                Some(Ok(Message::Binary(data))) if data.as_ref() == [0x0B, 0, 0, 26]));
        }
        drop(held);
        // Release the test-owned driver too; logical retirement alone does not
        // promise physical closure while external Arcs still retain the sink.
        drop(before);
        drop(old.sink);
        // No old NAK/queued heartbeat may flush after lock release.
        assert!(!matches!(
            poll_io(old.ws.next()).await,
            Some(Ok(Message::Binary(_)))
        ));
        poll_io(hub.hub.stop()).await;
        assert_eq!(hub.active.load(Ordering::Acquire), 0);
        assert!(hub.clients.lock().await.is_empty());
        assert_eq!(hub.hub.tasks.len(), 0);
        tokio::time::resume();
    }
}
