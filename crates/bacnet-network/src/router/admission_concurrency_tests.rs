//! Several ports, or several threads, racing one local queue: the shared
//! capacity and the depth accounting stay exact.

use super::*;

#[tokio::test]
async fn concurrent_ports_share_one_capacity_and_exact_depth_during_receive() {
    let (ports, mut peers) = fixture(4);
    let (mut router, mut apdus) = launch(ports, RouterOptions::new().track_admission()).await;
    let counters = apdus.counters();
    drain_announcements(&mut peers).await;
    let start = Arc::new(tokio::sync::Barrier::new(4));
    let mut producers = Vec::new();
    for (port, mut peer) in peers.into_iter().enumerate() {
        let start = Arc::clone(&start);
        producers.push(tokio::spawn(async move {
            start.wait().await;
            for id in 0..256 {
                peer.tx
                    .send(incoming(None, (port * 256 + id) as u16))
                    .await
                    .unwrap();
            }
            barrier(&mut peer).await;
            peer
        }));
    }
    let mut peers = Vec::new();
    for producer in producers {
        peers.push(
            timeout(Duration::from_secs(5), producer)
                .await
                .unwrap()
                .unwrap(),
        );
    }
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            current_depth: 256,
            high_water: 256,
            full_drops: 768,
            fairness_drops: 0,
            closed_drops: 0,
        }
    );
    let mut seen = std::collections::HashSet::new();
    while let Ok(apdu) = apdus.try_recv() {
        assert!(seen.insert(id_of(&apdu.apdu)));
    }
    assert_eq!(seen.len(), 256);
    assert_eq!(counters.snapshot().current_depth, 0);

    // Four live senders race the accounting receiver. At most 256 total items
    // arrive, so all must be received even with adversarial scheduling.
    let mut producers = Vec::new();
    for (port, mut peer) in peers.into_iter().enumerate() {
        producers.push(tokio::spawn(async move {
            for id in 0..64 {
                peer.tx
                    .send(incoming(None, (port * 64 + id) as u16))
                    .await
                    .unwrap();
            }
            barrier(&mut peer).await;
        }));
    }
    seen.clear();
    for _ in 0..256 {
        let apdu = timeout(Duration::from_secs(5), apdus.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(seen.insert(id_of(&apdu.apdu)));
        assert!(counters.snapshot().current_depth <= 256);
    }
    for producer in producers {
        timeout(Duration::from_secs(5), producer)
            .await
            .unwrap()
            .unwrap();
    }
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            high_water: 256,
            full_drops: 768,
            ..Default::default()
        }
    );
    router.stop().await.unwrap();
}

#[test]
fn cloned_senders_account_exactly_across_threads_and_concurrent_dequeues() {
    let (tx, rx, counters) = AdmissionReceiver::<u16>::channel(true);
    let mut apdus = AdmissionReceiver::from_parts(rx, counters.clone());
    let start = std::sync::Barrier::new(4);
    std::thread::scope(|scope| {
        for port in 0..4 {
            let tx = tx.clone();
            let start = &start;
            scope.spawn(move || {
                start.wait();
                for id in 0..256 {
                    let _ = tx.try_send(port * 256 + id);
                }
            });
        }
    });
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            current_depth: 256,
            high_water: 256,
            full_drops: 768,
            fairness_drops: 0,
            closed_drops: 0,
        }
    );
    for _ in 0..256 {
        apdus.try_recv().unwrap();
    }

    let start = std::sync::Barrier::new(5);
    std::thread::scope(|scope| {
        for port in 0..4 {
            let tx = tx.clone();
            let start = &start;
            scope.spawn(move || {
                start.wait();
                for id in 0..64 {
                    tx.try_send(port * 64 + id).unwrap();
                }
            });
        }
        start.wait();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut seen = std::collections::HashSet::new();
        while seen.len() < 256 {
            match apdus.try_recv() {
                Ok(id) => assert!(seen.insert(id)),
                Err(mpsc::error::TryRecvError::Empty) => {
                    assert!(std::time::Instant::now() < deadline, "sender stalled");
                    std::thread::yield_now();
                }
                Err(e) => panic!("unexpected receive error: {e}"),
            }
            assert!(counters.snapshot().current_depth <= 256);
        }
    });
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            high_water: 256,
            full_drops: 768,
            ..Default::default()
        }
    );
}
