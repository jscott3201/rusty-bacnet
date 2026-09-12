use super::*;

fn keyed_local(branch: LocalBranch, port: usize, source: &[u8], id: u16) -> ReceivedNpdu {
    let mut arrival = local(branch, port, id);
    arrival.source_mac = MacAddr::from_slice(source);
    arrival
}

fn assert_keyed_apdu(
    apdu: &ReceivedApdu,
    branch: LocalBranch,
    port: usize,
    source: &[u8],
    id: u16,
) {
    assert_eq!(apdu.apdu.as_ref(), id.to_be_bytes());
    assert_eq!(apdu.ingress_network, Some(network(port)));
    assert_eq!(apdu.source_mac.as_slice(), source);
    // Identical routed source on every port: it must not become the quota key.
    assert_eq!(apdu.source_network, Some(address(300, &[0x55])));
    assert!(apdu.link_layer_group);
    assert_eq!(apdu.is_group, !matches!(branch, LocalBranch::DadrMatch));
    assert_eq!(apdu.data_attributes, incoming(None, id).data_attributes);
}

async fn keyed_drop(peers: &mut [Peer], branch: LocalBranch, source: &[u8]) {
    let (reply_tx, reply_rx) = oneshot::channel();
    let mut arrival = keyed_local(branch, 0, source, 256);
    arrival.reply_tx = Some(reply_tx);
    peers[0].tx.send(arrival).await.unwrap();
    branch_forward(peers, branch, 0, 256).await;
    forwarding_progress(peers, 0).await;
    assert!(timeout(Duration::from_secs(2), reply_rx)
        .await
        .expect("admission drop retained reply sender")
        .is_err());
    assert_quiet(peers);
}

#[tokio::test(start_paused = true)]
async fn each_branch_caps_a_flood_and_same_mac_on_another_port_has_its_own_quota() {
    for branch in BRANCHES {
        let (ports, mut peers) = fixture(2);
        let (mut router, mut apdus) = BACnetRouter::start_with_admission(ports).await.unwrap();
        let counters = apdus.counters();
        drain_announcements(&mut peers).await;
        for id in 0..256 {
            let (reply_tx, reply_rx) = oneshot::channel();
            let mut arrival = keyed_local(branch, 0, &[2], id);
            arrival.reply_tx = Some(reply_tx);
            peers[0].tx.send(arrival).await.unwrap();
            branch_forward(&mut peers, branch, 0, id).await;
            if id >= 16 {
                assert!(timeout(Duration::from_secs(2), reply_rx)
                    .await
                    .expect("fairness drop retained reply sender")
                    .is_err());
            }
        }
        barrier(&mut peers[0]).await;
        assert_eq!(
            counters.snapshot(),
            QueueAdmissionSnapshot {
                current_depth: 16,
                high_water: 16,
                fairness_drops: 240,
                ..Default::default()
            }
        );
        // MACs are link-local: port B must make progress without draining A.
        for id in 0..17 {
            peers[1]
                .tx
                .send(keyed_local(branch, 1, &[2], id))
                .await
                .unwrap();
            branch_forward(&mut peers, branch, 1, id).await;
        }
        barrier(&mut peers[1]).await;
        // Nor may the quota accidentally become a per-port-only quota.
        for id in 0..16 {
            peers[0]
                .tx
                .send(keyed_local(branch, 0, &[3], id))
                .await
                .unwrap();
            branch_forward(&mut peers, branch, 0, id).await;
        }
        barrier(&mut peers[0]).await;
        assert_eq!(
            counters.snapshot(),
            QueueAdmissionSnapshot {
                current_depth: 48,
                high_water: 48,
                fairness_drops: 241,
                ..Default::default()
            }
        );
        for (port, source) in [(0, 2), (1, 2), (0, 3)] {
            for id in 0..16 {
                let apdu = apdus.try_recv().unwrap();
                assert_keyed_apdu(&apdu, branch, port, &[source], id);
            }
        }
        assert_eq!(counters.snapshot().current_depth, 0);
        assert_quiet(&mut peers); // Only pre-existing broadcast forwarding occurred.
        router.stop().await;
    }
}

#[tokio::test(start_paused = true)]
async fn recv_and_try_recv_replenish_only_the_exact_port_mac_key_one_for_one() {
    let (ports, mut peers) = fixture(2);
    let (mut router, mut apdus) = BACnetRouter::start_with_admission(ports).await.unwrap();
    let counters = apdus.counters();
    drain_announcements(&mut peers).await;
    for (port, peer) in peers.iter_mut().enumerate() {
        for id in 0..16 {
            peer.tx
                .send(keyed_local(LocalBranch::NoDnet, port, &[2], id))
                .await
                .unwrap();
        }
        barrier(peer).await;
    }
    let retained = [apdus.recv().await.unwrap(), apdus.try_recv().unwrap()];
    assert_eq!(counters.snapshot().current_depth, 30);
    for id in 16..19 {
        peers[0]
            .tx
            .send(keyed_local(LocalBranch::NoDnet, 0, &[2], id))
            .await
            .unwrap();
    }
    barrier(&mut peers[0]).await;
    peers[1]
        .tx
        .send(keyed_local(LocalBranch::NoDnet, 1, &[2], 16))
        .await
        .unwrap();
    barrier(&mut peers[1]).await;
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            current_depth: 32,
            high_water: 32,
            fairness_drops: 2,
            ..Default::default()
        }
    );
    for (port, ids) in [(0, 2..16), (1, 0..16), (0, 16..18)] {
        for id in ids {
            let apdu = if id % 2 == 0 {
                apdus.recv().await.unwrap()
            } else {
                apdus.try_recv().unwrap()
            };
            assert_keyed_apdu(&apdu, LocalBranch::NoDnet, port, &[2], id);
        }
    }
    assert_eq!(
        apdus.try_recv().unwrap_err(),
        mpsc::error::TryRecvError::Empty
    );
    assert!(timeout(Duration::from_secs(1), apdus.recv()).await.is_err());
    // Last dequeue removes the key; empty/cancelled receives cannot free extra
    // slots. Retaining or mutating returned APDUs cannot affect queued keys.
    for (port, peer) in peers.iter_mut().enumerate() {
        for id in 32..49 {
            peer.tx
                .send(keyed_local(LocalBranch::NoDnet, port, &[2], id))
                .await
                .unwrap();
        }
        barrier(peer).await;
    }
    assert_eq!(counters.snapshot().current_depth, 32);
    assert_eq!(counters.snapshot().fairness_drops, 4);
    assert_keyed_apdu(&retained[0], LocalBranch::NoDnet, 0, &[2], 0);
    assert_keyed_apdu(&retained[1], LocalBranch::NoDnet, 0, &[2], 1);
    router.stop().await;
    for port in 0..2 {
        for id in 32..48 {
            assert_keyed_apdu(
                &apdus.recv().await.unwrap(),
                LocalBranch::NoDnet,
                port,
                &[2],
                id,
            );
        }
    }
    assert!(apdus.recv().await.is_none());
    assert_eq!(counters.snapshot().current_depth, 0);
}

#[tokio::test(start_paused = true)]
async fn closed_precedes_fairness_which_precedes_full_in_all_local_branches() {
    for branch in BRANCHES {
        let (ports, mut peers) = fixture(2);
        let (mut router, mut apdus) = BACnetRouter::start_with_admission(ports).await.unwrap();
        let counters = apdus.counters();
        drain_announcements(&mut peers).await;
        let accepted_reply = fill(&mut peers).await;
        keyed_drop(&mut peers, branch, &[0xA0]).await; // Quota AND global Full.
        keyed_drop(&mut peers, branch, &[2]).await; // Within quota, global Full.
        assert_eq!(
            counters.snapshot(),
            QueueAdmissionSnapshot {
                current_depth: 256,
                high_water: 256,
                fairness_drops: 1,
                full_drops: 1,
                closed_drops: 0,
            }
        );
        apdus.close();
        keyed_drop(&mut peers, branch, &[0xA0]).await;
        keyed_drop(&mut peers, branch, &[2]).await;
        assert_eq!(
            counters.snapshot(),
            QueueAdmissionSnapshot {
                current_depth: 256,
                high_water: 256,
                fairness_drops: 1,
                full_drops: 1,
                closed_drops: 2,
            }
        );
        drop(apdus);
        assert!(accepted_reply.await.is_err());
        assert_eq!(counters.snapshot().current_depth, 0);
        router.stop().await;
    }
}

#[tokio::test(start_paused = true)]
async fn legacy_local_queue_accepts_256_from_one_port_mac_without_a_quota() {
    let (ports, mut peers) = fixture(2);
    let (mut router, mut apdus): (_, mpsc::Receiver<ReceivedApdu>) =
        BACnetRouter::start(ports).await.unwrap();
    drain_announcements(&mut peers).await;
    for id in 0..256 {
        peers[0]
            .tx
            .send(keyed_local(LocalBranch::NoDnet, 0, &[2], id))
            .await
            .unwrap();
    }
    barrier(&mut peers[0]).await;
    keyed_drop(&mut peers, LocalBranch::NoDnet, &[2]).await;
    assert_eq!(apdus.len(), 256);
    for id in 0..256 {
        assert_keyed_apdu(&apdus.try_recv().unwrap(), LocalBranch::NoDnet, 0, &[2], id);
    }
    router.stop().await;
}
