//! Closing, dropping and stopping around a full local queue: queued APDUs stay
//! drainable or are released, and later arrivals count as closed drops.

use super::*;

#[tokio::test(start_paused = true)]
async fn closing_full_local_queue_counts_each_closed_arrival_and_preserves_drain() {
    let (ports, mut peers) = fixture(2);
    let (mut router, mut apdus) = launch(ports, RouterOptions::new().track_admission()).await;
    let counters = apdus.counters();
    drain_announcements(&mut peers).await;
    let mut accepted_reply = fill(&mut peers).await;
    apdus.close();
    for branch in BRANCHES {
        for port in 0..2 {
            dropped_arrival(&mut peers, branch, port).await;
        }
    }
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            current_depth: 256,
            high_water: 256,
            full_drops: 0,
            fairness_drops: 0,
            closed_drops: 8,
        }
    );
    assert_eq!(
        accepted_reply.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    );
    for id in 0..256 {
        let apdu = if id % 2 == 0 {
            apdus.recv().await.unwrap()
        } else {
            apdus.try_recv().unwrap()
        };
        assert_apdu(&apdu, LocalBranch::NoDnet, usize::from(id / 128), id);
        assert_eq!(counters.snapshot().current_depth, 255 - usize::from(id));
    }
    assert!(accepted_reply.await.is_err());
    assert!(apdus.recv().await.is_none());
    assert!(matches!(
        apdus.try_recv(),
        Err(mpsc::error::TryRecvError::Disconnected)
    ));
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            high_water: 256,
            closed_drops: 8,
            ..Default::default()
        }
    );
    router.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn dropping_local_receiver_releases_queued_replies_and_keeps_dispatch_alive() {
    let (ports, mut peers) = fixture(2);
    let (mut router, apdus) = launch(ports, RouterOptions::new().track_admission()).await;
    let counters = apdus.counters();
    drain_announcements(&mut peers).await;
    let accepted_reply = fill(&mut peers).await;
    drop(apdus);
    assert!(accepted_reply.await.is_err());
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            high_water: 256,
            ..Default::default()
        }
    );
    for port in 0..2 {
        dropped_arrival(&mut peers, LocalBranch::NoDnet, port).await;
    }
    assert_eq!(counters.snapshot().closed_drops, 2);
    router.stop().await.unwrap();
    drop(router);
    assert_eq!(counters.snapshot().current_depth, 0);
}

#[tokio::test(start_paused = true)]
async fn stop_with_full_local_queue_is_bounded_and_leaves_items_drainable() {
    let (ports, mut peers) = fixture(2);
    let (mut router, mut apdus) = launch(ports, RouterOptions::new().track_admission()).await;
    let counters = apdus.counters();
    drain_announcements(&mut peers).await;
    let mut accepted_reply = fill(&mut peers).await;
    timeout(Duration::from_secs(2), router.stop())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        accepted_reply.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    );
    assert_eq!(counters.snapshot().current_depth, 256);
    assert!(peers.iter().all(|peer| peer.tx.is_closed()));
    for id in 0..256 {
        assert_apdu(
            &apdus.recv().await.unwrap(),
            LocalBranch::NoDnet,
            usize::from(id / 128),
            id,
        );
    }
    assert!(apdus.recv().await.is_none());
    assert!(accepted_reply.await.is_err());
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            high_water: 256,
            ..Default::default()
        }
    );
}
