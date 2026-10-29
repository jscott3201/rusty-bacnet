//! The router's shared local APDU queue: what a full queue drops, what an
//! admitted APDU keeps, and the raw receiver's policy. The modules below cover
//! the receiver's lifecycle, concurrent producers and the per-source quota.
//! All of them run a real router through
//! [`ingress_harness`](super::ingress_harness).

use super::ingress_harness::*;
use super::*;
use crate::layer::QueueAdmissionSnapshot;
use bacnet_transport::port::ReceivedNpdu;
use tokio::sync::oneshot;
use tokio::time::timeout;

#[tokio::test(start_paused = true)]
async fn full_shared_local_queue_drops_arrivals_in_all_four_branches_and_keeps_forwarding() {
    for branch in BRANCHES {
        let (ports, mut peers) = fixture(2);
        let (mut router, mut apdus) = launch(ports, RouterOptions::new().track_admission()).await;
        let counters = apdus.counters();
        drain_announcements(&mut peers).await;
        let mut accepted_reply = fill(&mut peers).await;
        for port in 0..2 {
            dropped_arrival(&mut peers, branch, port).await;
            assert_eq!(
                counters.snapshot(),
                QueueAdmissionSnapshot {
                    current_depth: 256,
                    high_water: 256,
                    full_drops: port as u64 + 1,
                    fairness_drops: 0,
                    closed_drops: 0,
                }
            );
        }
        assert_eq!(
            accepted_reply.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        );
        for id in 0..256 {
            let apdu = apdus.try_recv().unwrap();
            assert_apdu(&apdu, LocalBranch::NoDnet, usize::from(id / 128), id);
            if id == 0 {
                apdu.reply_tx
                    .unwrap()
                    .send(Bytes::from_static(b"reply"))
                    .unwrap();
            }
            assert_eq!(counters.snapshot().current_depth, 255 - usize::from(id));
        }
        assert_eq!(accepted_reply.await.unwrap(), Bytes::from_static(b"reply"));
        assert!(matches!(
            apdus.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        peers[0].tx.send(incoming(None, 257)).await.unwrap();
        let apdu = timeout(Duration::from_secs(2), apdus.recv())
            .await
            .unwrap()
            .unwrap();
        assert_apdu(&apdu, LocalBranch::NoDnet, 0, 257);
        assert_eq!(
            counters.snapshot(),
            QueueAdmissionSnapshot {
                high_water: 256,
                full_drops: 2,
                ..Default::default()
            }
        );
        router.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn admitted_local_branches_preserve_metadata_and_reply_ownership() {
    let (ports, mut peers) = fixture(2);
    let (mut router, mut apdus) = launch(ports, RouterOptions::new().track_admission()).await;
    drain_announcements(&mut peers).await;
    for branch in BRANCHES {
        let (reply_tx, mut reply_rx) = oneshot::channel();
        let mut arrival = local(branch, 0, 42);
        arrival.reply_tx = Some(reply_tx);
        peers[0].tx.send(arrival).await.unwrap();
        branch_forward(&mut peers, branch, 0, 42).await;
        barrier(&mut peers[0]).await;
        assert_eq!(apdus.counters().snapshot().current_depth, 1);
        let apdu = apdus.recv().await.unwrap();
        assert_apdu(&apdu, branch, 0, 42);
        assert_eq!(apdu.clone().ingress_network, apdu.ingress_network);
        if matches!(branch, LocalBranch::RemoteBroadcast) {
            assert!(apdu.reply_tx.is_none());
            assert!(reply_rx.await.is_err());
        } else {
            assert_eq!(
                reply_rx.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            );
            apdu.reply_tx
                .unwrap()
                .send(Bytes::from_static(b"reply"))
                .unwrap();
            assert_eq!(reply_rx.await.unwrap(), Bytes::from_static(b"reply"));
        }
        assert_eq!(
            apdus.counters().snapshot(),
            QueueAdmissionSnapshot {
                high_water: 1,
                ..Default::default()
            }
        );
    }
    assert_quiet(&mut peers);
    router.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn legacy_local_receiver_retains_type_and_nonblocking_full_closed_policy() {
    let (ports, mut peers) = fixture(2);
    let (mut router, mut apdus): (_, mpsc::Receiver<ReceivedApdu>) =
        launch(ports, RouterOptions::new()).await;
    drain_announcements(&mut peers).await;
    let accepted_reply = fill(&mut peers).await;
    for branch in BRANCHES {
        dropped_arrival(&mut peers, branch, 0).await;
    }
    assert_eq!(apdus.len(), 256);
    for id in 0..256 {
        assert_apdu(
            &apdus.try_recv().unwrap(),
            LocalBranch::NoDnet,
            usize::from(id / 128),
            id,
        );
    }
    assert!(accepted_reply.await.is_err());
    apdus.close();
    for branch in BRANCHES {
        dropped_arrival(&mut peers, branch, 1).await;
    }
    assert!(apdus.recv().await.is_none());
    router.stop().await.unwrap();
}

#[path = "admission_lifecycle_tests.rs"]
mod lifecycle_tests;

#[path = "admission_concurrency_tests.rs"]
mod concurrency_tests;

#[path = "fairness_tests.rs"]
mod fairness_tests;
